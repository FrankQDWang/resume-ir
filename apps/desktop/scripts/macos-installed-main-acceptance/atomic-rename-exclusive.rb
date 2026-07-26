#!/usr/bin/ruby

require "fiddle/import"

module LibSystem
  extend Fiddle::Importer
  dlload "/usr/lib/libSystem.B.dylib"
  extern "int renamex_np(const char *, const char *, unsigned int)"
end

RENAME_EXCL = 0x00000004
RENAME_NOFOLLOW_ANY = 0x00000010

unless ARGV.length == 2 &&
       ARGV.all? { |value| value.start_with?("/") && !value.include?("\0") } &&
       File.dirname(ARGV[0]) == File.dirname(ARGV[1])
  exit 64
end

result = LibSystem.renamex_np(
  ARGV[0],
  ARGV[1],
  RENAME_EXCL | RENAME_NOFOLLOW_ANY,
)
exit 0 if result == 0

case Fiddle.last_error
when Errno::EACCES::Errno, Errno::EPERM::Errno
  exit 73
when Errno::EEXIST::Errno, Errno::ENOTEMPTY::Errno
  exit 74
else
  exit 70
end
