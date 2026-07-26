import http from "node:http";
import { randomBytes } from "node:crypto";
import { constants } from "node:fs";
import { open, rename, rm } from "node:fs/promises";
import path from "node:path";

import {
  AUTH_FILE,
  CONTROL_PUBLICATION_TIMEOUT_MS,
  ENDPOINT_FILE,
  POLL_MS,
  AcceptanceError,
  fail,
  throwIfAborted,
  wait,
} from "./core.mjs";
import {
  readPrivateJson,
  requirePrivateFile,
  requireSecureDirectory,
} from "./filesystem-cow.mjs";
import { readDaemonConnection } from "./ipc-contracts.mjs";

function privateJsonSource(value) {
  return `${JSON.stringify(value)}\n`;
}

async function persistPrivateJson(directory, fileName, value) {
  await requireSecureDirectory(directory, { privateMode: true });
  const target = path.join(directory, fileName);
  const temporary = path.join(
    directory,
    `.${fileName}.acceptance-${randomBytes(16).toString("hex")}`,
  );
  const source = privateJsonSource(value);
  let directoryHandle;
  let temporaryHandle;
  try {
    temporaryHandle = await open(temporary, "wx", 0o600);
    await temporaryHandle.writeFile(source, "utf8");
    await temporaryHandle.sync();
    await temporaryHandle.close();
    temporaryHandle = undefined;
    await rename(temporary, target);
    directoryHandle = await open(directory, constants.O_RDONLY);
    await directoryHandle.sync();
    await requirePrivateFile(target, { maxBytes: 16 * 1024 });
    return source;
  } catch (error) {
    await temporaryHandle?.close().catch(() => {});
    await rm(temporary, { force: true }).catch(() => {});
    if (error instanceof AcceptanceError) throw error;
    fail("control_fixture_invalid");
  } finally {
    await directoryHandle?.close().catch(() => {});
  }
}

function v3Endpoints(origin, launchId, instanceId) {
  return {
    schema_version: "resume-ir.daemon-ipc.v3",
    launch_id: launchId,
    instance_id: instanceId,
    owner_mode: "desktop_supervised",
    status: `${origin}/status`,
    diagnostics: `${origin}/diagnostics`,
    imports: `${origin}/imports`,
    import_cancel: `${origin}/imports/cancel`,
    import_control: `${origin}/imports/control`,
    import_progress: `${origin}/imports/progress`,
    search: `${origin}/search`,
    search_batch: `${origin}/search/batch`,
    details: `${origin}/details`,
    delete: `${origin}/delete`,
  };
}

function v3Auth(launchId, instanceId, token) {
  return {
    schema_version: "resume-ir.daemon-auth.v3",
    launch_id: launchId,
    instance_id: instanceId,
    token,
  };
}

export async function prepareStaleControlFixture(dataDir) {
  const launchId = randomBytes(32).toString("hex");
  const instanceId = randomBytes(32).toString("hex");
  const endpointSource = await persistPrivateJson(dataDir, ENDPOINT_FILE, {
    schema_version: "resume-ir.daemon-ipc.v2",
    instance_id: instanceId,
    owner_mode: "standalone",
    status: "http://127.0.0.1:9/status",
    diagnostics: "http://127.0.0.1:9/diagnostics",
  });
  const authSource = await persistPrivateJson(dataDir, AUTH_FILE, {
    schema_version: "resume-ir.daemon-auth.v2",
    instance_id: instanceId,
    token: randomBytes(32).toString("hex"),
  });
  return Object.freeze({
    authSource,
    dataDir,
    endpointSource,
    injectedLaunchId: launchId,
    kind: "stale",
  });
}

async function listenForeignEndpoint() {
  let requests = 0;
  const server = http.createServer((_request, response) => {
    requests += 1;
    response.writeHead(503, { "Content-Type": "application/json" });
    response.end('{"status":"foreign"}\n');
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      server.off("error", reject);
      resolve();
    });
  }).catch(() => fail("foreign_control_fixture_invalid"));
  const address = server.address();
  if (
    address === null ||
    typeof address === "string" ||
    address.address !== "127.0.0.1" ||
    !Number.isSafeInteger(address.port)
  ) {
    server.close();
    fail("foreign_control_fixture_invalid");
  }
  return {
    close: () =>
      new Promise((resolve) => {
        if (!server.listening) return resolve();
        server.close(() => resolve());
      }),
    isListening: () => server.listening,
    origin: `http://127.0.0.1:${address.port}`,
    requestCount: () => requests,
  };
}

export async function prepareForeignControlFixture(dataDir) {
  const endpoint = await listenForeignEndpoint();
  const launchId = randomBytes(32).toString("hex");
  const instanceId = randomBytes(32).toString("hex");
  try {
    const endpointSource = await persistPrivateJson(
      dataDir,
      ENDPOINT_FILE,
      v3Endpoints(endpoint.origin, launchId, instanceId),
    );
    const authSource = await persistPrivateJson(
      dataDir,
      AUTH_FILE,
      v3Auth(launchId, instanceId, randomBytes(32).toString("hex")),
    );
    return {
      authSource,
      close: endpoint.close,
      dataDir,
      endpointSource,
      injectedLaunchId: launchId,
      isListening: endpoint.isListening,
      kind: "foreign",
      requestCount: endpoint.requestCount,
    };
  } catch (error) {
    await endpoint.close();
    throw error;
  }
}

export async function waitForControlReplacement(
  fixture,
  acceptCandidate,
  signal,
  timeoutMs = CONTROL_PUBLICATION_TIMEOUT_MS,
) {
  if (typeof acceptCandidate !== "function") {
    fail("control_fixture_invalid");
  }
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    throwIfAborted(signal);
    try {
      const [connection, endpoints, auth] = await Promise.all([
        readDaemonConnection(fixture.dataDir),
        readPrivateJson(path.join(fixture.dataDir, ENDPOINT_FILE), 16 * 1024),
        readPrivateJson(path.join(fixture.dataDir, AUTH_FILE), 16 * 1024),
      ]);
      if (
        connection.launchId !== fixture.injectedLaunchId &&
        endpoints.source !== fixture.endpointSource &&
        auth.source !== fixture.authSource &&
        (await acceptCandidate(connection))
      ) {
        return connection;
      }
    } catch (error) {
      if (
        error instanceof AcceptanceError &&
        error.code === "acceptance_interrupted"
      ) {
        throw error;
      }
    }
    await wait(POLL_MS);
  }
  fail("control_replacement_timeout");
}

export function validateForeignControlPreserved(fixture) {
  if (
    fixture.kind !== "foreign" ||
    fixture.requestCount() !== 0 ||
    fixture.isListening() !== true
  ) {
    fail("foreign_control_touched");
  }
  return true;
}
