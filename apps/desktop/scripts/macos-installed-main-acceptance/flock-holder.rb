# frozen_string_literal: true

READY = "resume-ir.installed-main-publication-lock.ready.v1\n"

path = ARGV.first
valid_path = ARGV.length == 1 && path&.start_with?("/") &&
             !path.include?("\0") && path.bytesize <= 4096
exit 64 unless valid_path

begin
  before = File.lstat(path)
  valid_file = before.file? && !before.symlink? && before.uid == Process.uid &&
               before.nlink == 1 && (before.mode & 0o777) == 0o600 &&
               before.size.zero?
  exit 70 unless valid_file

  File.open(path, File::RDWR | File::NOFOLLOW) do |file|
    opened = file.stat
    exit 70 unless opened.dev == before.dev && opened.ino == before.ino
    exit 75 unless file.flock(File::LOCK_EX | File::LOCK_NB)

    STDOUT.sync = true
    STDOUT.write(READY)
    STDIN.read
  end
rescue StandardError
  exit 70
end
