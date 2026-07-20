import { toolSucceeded } from "./bounded-process.mjs";
import {
  RUN_ID,
  TOOL_TIMEOUT_MS,
  fail,
  validAbsolutePath,
} from "./core.mjs";

export const PROCESS_START_TIME =
  /^[A-Z][a-z]{2} [A-Z][a-z]{2} [ 0-3][0-9] [0-2][0-9]:[0-5][0-9]:[0-6][0-9] [0-9]{4}$/;

export function validateDurableProcessRecord(value) {
  if (
    value === null ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    !Number.isSafeInteger(value.pid) ||
    value.pid <= 1 ||
    !Number.isSafeInteger(value.pgid) ||
    value.pgid <= 1 ||
    !PROCESS_START_TIME.test(value.start_time ?? "") ||
    !validAbsolutePath(value.executable) ||
    !RUN_ID.test(value.session_authority ?? "")
  ) {
    fail("workspace_marker_invalid");
  }
  return value;
}

export async function readProcessStartTime(pid, runTool) {
  if (!Number.isSafeInteger(pid) || pid <= 1 || typeof runTool !== "function") {
    fail("process_identity_invalid");
  }
  const result = await runTool(
    "/bin/ps",
    ["-p", String(pid), "-o", "lstart="],
    { timeoutMs: TOOL_TIMEOUT_MS },
  );
  const source = result?.stdout?.trimEnd();
  if (
    !toolSucceeded(result) ||
    result.stderr !== "" ||
    typeof source !== "string" ||
    source.includes("\n") ||
    !PROCESS_START_TIME.test(source)
  ) {
    fail("process_identity_invalid");
  }
  return source;
}

export async function requireRecordedProcessStart(record, runTool, code) {
  try {
    if ((await readProcessStartTime(record.pid, runTool)) !== record.start_time) {
      fail(code);
    }
  } catch {
    fail(code);
  }
}
