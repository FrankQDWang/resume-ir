# frozen_string_literal: true

require "fiddle/import"

module LibSystem
  extend Fiddle::Importer
  dlload "/usr/lib/libSystem.B.dylib"
  extern "int clonefile(const char *, const char *, int)"
end

source, destination = ARGV
valid = ARGV.length == 2 && [source, destination].all? do |candidate|
  candidate&.start_with?("/") && !candidate.include?("\0") && candidate.bytesize <= 4096
end
exit 64 unless valid

# CLONE_NOFOLLOW makes a source pathname replacement fail rather than following it.
exit(LibSystem.clonefile(source, destination, 1).zero? ? 0 : 70)
