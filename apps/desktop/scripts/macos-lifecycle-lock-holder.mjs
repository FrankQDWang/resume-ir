const READY_LINE = "resume-ir.macos-lifecycle-lock.ready.v1\n";

let finished = false;

function finish(exitCode = 0) {
  if (finished) return;
  finished = true;
  process.exitCode = exitCode;
  process.stdin.destroy();
}

process.stdin.once("end", () => finish());
process.stdin.once("close", () => finish());
process.stdin.once("error", () => finish());
process.stdout.once("error", () => finish());

process.stdout.write(READY_LINE, (error) => {
  if (error) {
    finish(70);
    return;
  }
  process.stdin.resume();
});
