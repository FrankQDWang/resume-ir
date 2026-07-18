import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { inflateSync } from "node:zlib";

const PNG_SIGNATURE = Buffer.from([
  0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a,
]);

function paeth(left, above, upperLeft) {
  const estimate = left + above - upperLeft;
  const leftDistance = Math.abs(estimate - left);
  const aboveDistance = Math.abs(estimate - above);
  const upperLeftDistance = Math.abs(estimate - upperLeft);
  if (leftDistance <= aboveDistance && leftDistance <= upperLeftDistance) {
    return left;
  }
  return aboveDistance <= upperLeftDistance ? above : upperLeft;
}

function decodeRgbaPng(png) {
  assert.deepEqual(png.subarray(0, 8), PNG_SIGNATURE);
  let offset = 8;
  let width;
  let height;
  const compressed = [];

  while (offset < png.length) {
    const length = png.readUInt32BE(offset);
    const type = png.toString("ascii", offset + 4, offset + 8);
    const data = png.subarray(offset + 8, offset + 8 + length);
    offset += length + 12;
    if (type === "IHDR") {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      assert.equal(data[8], 8, "icon PNG must use 8-bit channels");
      assert.equal(data[9], 6, "icon PNG must use RGBA pixels");
      assert.equal(data[12], 0, "icon PNG must not be interlaced");
    } else if (type === "IDAT") {
      compressed.push(data);
    } else if (type === "IEND") {
      break;
    }
  }

  assert.ok(width && height && compressed.length > 0);
  const packed = inflateSync(Buffer.concat(compressed));
  const stride = width * 4;
  assert.equal(packed.length, (stride + 1) * height);
  const pixels = Buffer.alloc(stride * height);

  for (let y = 0; y < height; y += 1) {
    const filter = packed[y * (stride + 1)];
    const inputStart = y * (stride + 1) + 1;
    const outputStart = y * stride;
    for (let x = 0; x < stride; x += 1) {
      const encoded = packed[inputStart + x];
      const left = x >= 4 ? pixels[outputStart + x - 4] : 0;
      const above = y > 0 ? pixels[outputStart + x - stride] : 0;
      const upperLeft = y > 0 && x >= 4
        ? pixels[outputStart + x - stride - 4]
        : 0;
      let prediction = 0;
      if (filter === 1) prediction = left;
      else if (filter === 2) prediction = above;
      else if (filter === 3) prediction = Math.floor((left + above) / 2);
      else if (filter === 4) prediction = paeth(left, above, upperLeft);
      else assert.equal(filter, 0, `unsupported PNG filter ${filter}`);
      pixels[outputStart + x] = (encoded + prediction) & 0xff;
    }
  }

  return { width, height, pixels };
}

test("macOS icon has a standard rounded silhouette with transparent corners", async () => {
  const icon = decodeRgbaPng(
    await readFile(new URL("../src-tauri/icons/icon.png", import.meta.url)),
  );
  assert.deepEqual([icon.width, icon.height], [1024, 1024]);

  const alphaAt = (x, y) => icon.pixels[(y * icon.width + x) * 4 + 3];
  assert.equal(alphaAt(0, 0), 0);
  assert.equal(alphaAt(64, 64), 0);
  assert.equal(alphaAt(512, 16), 255);
  assert.equal(alphaAt(16, 512), 255);
  assert.equal(alphaAt(512, 512), 255);

  let transparentPixels = 0;
  for (let index = 3; index < icon.pixels.length; index += 4) {
    if (icon.pixels[index] === 0) transparentPixels += 1;
  }
  const transparentRatio = transparentPixels / (icon.width * icon.height);
  assert.ok(transparentRatio > 0.05, "icon corners must be visibly transparent");
  assert.ok(transparentRatio < 0.12, "icon artwork must retain standard optical size");
});
