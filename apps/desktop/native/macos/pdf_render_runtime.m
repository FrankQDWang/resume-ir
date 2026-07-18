#import <CoreGraphics/CoreGraphics.h>
#import <Foundation/Foundation.h>

#include <errno.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

static const unsigned long long kMaximumInputBytes = 64ULL * 1024ULL * 1024ULL;
static const long kMaximumPageNumber = 512;
static const long kMinimumDpi = 72;
static const long kMaximumDpi = 600;
static const size_t kMaximumPixels = 10000000;

typedef enum {
  RenderResultSuccess = 0,
  RenderResultUnavailable = 1,
  RenderResultInvalidRequest = 2,
  RenderResultOverBudget = 3,
} RenderResult;

static const char *required_environment(const char *name) {
  const char *value = getenv(name);
  return value != NULL && value[0] != '\0' ? value : NULL;
}

static bool checked_integer(const char *name, long minimum, long maximum,
                            long *output) {
  const char *value = required_environment(name);
  if (value == NULL || strlen(value) > 8) {
    return false;
  }
  char *end = NULL;
  errno = 0;
  long parsed = strtol(value, &end, 10);
  if (errno != 0 || end == value || *end != '\0' || parsed < minimum ||
      parsed > maximum) {
    return false;
  }
  *output = parsed;
  return true;
}

static NSURL *checked_input_url(void) {
  const char *value = required_environment("RESUME_IR_PDF_RENDER_INPUT_PATH");
  if (value == NULL || value[0] != '/' || strlen(value) > 4096) {
    return nil;
  }
  struct stat metadata;
  if (lstat(value, &metadata) != 0 || !S_ISREG(metadata.st_mode) ||
      metadata.st_size <= 0 ||
      (unsigned long long)metadata.st_size > kMaximumInputBytes) {
    return nil;
  }
  char canonical[PATH_MAX];
  if (realpath(value, canonical) == NULL) {
    return nil;
  }
  return [NSURL fileURLWithPath:[NSString stringWithUTF8String:canonical]
                   isDirectory:NO];
}

static RenderResult render_pdf_page(void) {
  NSURL *input = checked_input_url();
  long page_number = 0;
  long dpi = 0;
  if (input == nil ||
      !checked_integer("RESUME_IR_PDF_RENDER_PAGE_NO", 1,
                       kMaximumPageNumber, &page_number) ||
      !checked_integer("RESUME_IR_PDF_RENDER_DPI", kMinimumDpi, kMaximumDpi,
                       &dpi)) {
    return RenderResultInvalidRequest;
  }

  CGDataProviderRef provider =
      CGDataProviderCreateWithURL((__bridge CFURLRef)input);
  CGPDFDocumentRef document =
      provider == NULL ? NULL : CGPDFDocumentCreateWithProvider(provider);
  CGDataProviderRelease(provider);
  size_t page_count = document == NULL ? 0 : CGPDFDocumentGetNumberOfPages(document);
  if (document == NULL || page_count == 0 ||
      page_count > (size_t)kMaximumPageNumber ||
      page_number > (long)page_count) {
    if (document != NULL) {
      CGPDFDocumentRelease(document);
    }
    return RenderResultUnavailable;
  }
  CGPDFPageRef page = CGPDFDocumentGetPage(document, (size_t)page_number);
  if (page == nil) {
    CGPDFDocumentRelease(document);
    return RenderResultUnavailable;
  }

  CGRect bounds = CGPDFPageGetBoxRect(page, kCGPDFMediaBox);
  if (!isfinite(bounds.size.width) || !isfinite(bounds.size.height) ||
      bounds.size.width <= 0 || bounds.size.height <= 0) {
    CGPDFDocumentRelease(document);
    return RenderResultInvalidRequest;
  }
  CGFloat scale = (CGFloat)dpi / 72.0;
  size_t width = (size_t)ceil(bounds.size.width * scale);
  size_t height = (size_t)ceil(bounds.size.height * scale);
  if (width == 0 || height == 0 || width > 10000 || height > 10000 ||
      width > kMaximumPixels / height) {
    CGPDFDocumentRelease(document);
    return RenderResultOverBudget;
  }

  size_t rgba_bytes = width * height * 4;
  unsigned char *pixels = malloc(rgba_bytes);
  if (pixels == NULL) {
    CGPDFDocumentRelease(document);
    return RenderResultUnavailable;
  }
  memset(pixels, 255, rgba_bytes);
  CGColorSpaceRef color_space = CGColorSpaceCreateDeviceRGB();
  CGContextRef context = CGBitmapContextCreate(
      pixels, width, height, 8, width * 4, color_space,
      (CGBitmapInfo)kCGImageAlphaNoneSkipLast);
  CGColorSpaceRelease(color_space);
  if (context == NULL) {
    free(pixels);
    CGPDFDocumentRelease(document);
    return RenderResultUnavailable;
  }
  CGContextSetRGBFillColor(context, 1, 1, 1, 1);
  CGContextFillRect(context, CGRectMake(0, 0, width, height));
  CGRect target = CGRectMake(0, 0, width, height);
  CGAffineTransform transform = CGPDFPageGetDrawingTransform(
      page, kCGPDFMediaBox, target, 0, true);
  CGContextConcatCTM(context, transform);
  CGContextDrawPDFPage(context, page);
  CGContextRelease(context);
  CGPDFDocumentRelease(document);

  char header[64];
  int header_bytes = snprintf(header, sizeof(header), "P6\n%zu %zu\n255\n",
                              width, height);
  size_t rgb_bytes = width * height * 3;
  unsigned char *output = malloc((size_t)header_bytes + rgb_bytes);
  if (header_bytes <= 0 || output == NULL) {
    free(pixels);
    free(output);
    return RenderResultUnavailable;
  }
  memcpy(output, header, (size_t)header_bytes);
  for (size_t row = 0; row < height; row++) {
    size_t source_row = height - row - 1;
    for (size_t column = 0; column < width; column++) {
      size_t source = (source_row * width + column) * 4;
      size_t destination = (row * width + column) * 3;
      output[header_bytes + destination] = pixels[source];
      output[header_bytes + destination + 1] = pixels[source + 1];
      output[header_bytes + destination + 2] = pixels[source + 2];
    }
  }
  free(pixels);
  size_t total = (size_t)header_bytes + rgb_bytes;
  bool written = fwrite(output, 1, total, stdout) == total && fflush(stdout) == 0;
  free(output);
  return written ? RenderResultSuccess : RenderResultUnavailable;
}

int main(int argc, const char *argv[]) {
  @autoreleasepool {
    if (argc != 1) {
      fputs("invalid renderer request\n", stderr);
      return RenderResultInvalidRequest;
    }
    RenderResult result = render_pdf_page();
    if (result == RenderResultInvalidRequest) {
      fputs("invalid renderer request\n", stderr);
    } else if (result == RenderResultOverBudget) {
      fputs("renderer request exceeded budget\n", stderr);
    } else if (result != RenderResultSuccess) {
      fputs("renderer unavailable\n", stderr);
    }
    return result;
  }
}
