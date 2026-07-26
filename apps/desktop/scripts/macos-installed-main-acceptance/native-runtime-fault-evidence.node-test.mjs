import assert from "node:assert/strict";
import { chmod, lstat, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { CLOSED_SYSTEM_TOOL_ENV } from "../macos-system-tools.mjs";
import {
  captureSyntheticFaultWitness,
  createOcrFaultFixture,
  readActiveClassifierEpoch,
  submitImportExpectedRejected,
  submitOcrBacklogImport,
  validateEmbeddingFaultDataPlane,
} from "./native-runtime-fault-evidence.mjs";
import { SYNTHETIC_CANARY_FILE_NAME } from "./synthetic-canary.mjs";

const REPO_ROOT = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../../..",
);
const TOKEN = "c".repeat(64);
const SELECTION = Object.freeze({
  doc_id: `doc_${"a".repeat(32)}`,
  version_id: `ver_${"b".repeat(32)}`,
  visible_epoch: 7,
});
const STAGES = Object.freeze({
  parse: 0,
  prefilter: 0,
  bm25: 0.2,
  ann: 0,
  fusion: 0,
  bulk_hydrate: 0.1,
  snippet: 0.1,
});

function searchResponse(request, hybridReasons = [
  "embedding_runtime_unavailable",
]) {
  const hybrid = request.payload.mode === "hybrid";
  return {
    schema_version: "resume-ir.search-response.v3",
    request_id: request.request_id,
    status: "ok",
    visible_epoch: SELECTION.visible_epoch,
    query_mode: hybrid ? "hybrid" : "keyword",
    partial: hybrid,
    partial_reasons: hybrid ? hybridReasons : [],
    latency_ms: 0.5,
    stage_latency_ms: STAGES,
    search_index: "available",
    result_count: 1,
    results: [
      {
        rank: 1,
        selection: SELECTION,
        file_name: SYNTHETIC_CANARY_FILE_NAME,
        snippet: "bounded synthetic snippet",
      },
    ],
  };
}

function detailResponse(request) {
  return {
    schema_version: "resume-ir.detail-response.v3",
    request_id: request.request_id,
    selection: request.selection,
    status: "ok",
    document: {
      source_byte_size: 128,
      parse_version: "parser-v1",
      schema_version: "schema-v29",
      language_set: ["en"],
      page_count: 1,
      quality_score: 0.95,
      fields_truncated: false,
      fields: [],
      snippet: "bounded synthetic detail",
    },
    limits: {
      max_fields: 256,
      max_response_bytes: 1024 * 1024,
    },
  };
}

async function startServer(context, responder) {
  const server = http.createServer(async (request, response) => {
    try {
      const chunks = [];
      for await (const chunk of request) chunks.push(chunk);
      const body = JSON.parse(Buffer.concat(chunks).toString("utf8"));
      const reply = await responder({
        body,
        path: request.url,
        token: request.headers.authorization,
      });
      response.writeHead(reply.status, { "Content-Type": "application/json" });
      response.end(JSON.stringify(reply.body));
    } catch {
      response.writeHead(500, { "Content-Type": "application/json" });
      response.end(JSON.stringify({ error: "fixture_failure" }));
    }
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  context.after(
    () => new Promise((resolve) => server.close(resolve)),
  );
  const address = server.address();
  const origin = `http://127.0.0.1:${address.port}`;
  return Object.freeze({
    token: TOKEN,
    urls: Object.freeze({
      details: new URL(`${origin}/details`),
      imports: new URL(`${origin}/imports`),
      search: new URL(`${origin}/search`),
    }),
  });
}

test("embedding fault proves exact keyword, lexical hybrid, and detail behavior", async (context) => {
  const connection = await startServer(context, ({ body, path: route, token }) => {
    assert.equal(token, `Bearer ${TOKEN}`);
    if (route === "/search") {
      return { body: searchResponse(body), status: 200 };
    }
    assert.equal(route, "/details");
    return { body: detailResponse(body), status: 200 };
  });
  const selection = await captureSyntheticFaultWitness(connection);
  assert.deepEqual(selection, SELECTION);
  assert.deepEqual(
    await validateEmbeddingFaultDataPlane(connection, selection),
    {
      detailAvailable: true,
      hybridLexicalPartial: true,
      keywordAvailable: true,
      selectionPreserved: true,
    },
  );
});

test("embedding fault rejects an unmarked hybrid lexical fallback", async (context) => {
  const connection = await startServer(context, ({ body, path: route }) => {
    if (route === "/search") {
      return { body: searchResponse(body, []), status: 200 };
    }
    return { body: detailResponse(body), status: 200 };
  });
  await assert.rejects(
    validateEmbeddingFaultDataPlane(connection, SELECTION),
    /optional_runtime_data_plane_invalid/,
  );
});

test("classifier gate accepts only the typed capability-unavailable response", async (context) => {
  const exact = await startServer(context, () => ({
    body: {
      schema_version: "resume-ir.error.v2",
      status: "error",
      error: {
        code: "CAPABILITY_UNAVAILABLE",
        action: "select_supported_mode",
        capability: "text_import",
        reason: "classifier_unavailable",
      },
    },
    status: 503,
  }));
  await submitImportExpectedRejected(exact, {
    roots: ["/private/synthetic"],
    profile: "explicit",
    max_files: 1,
  });

  const wrong = await startServer(context, () => ({
    body: {
      schema_version: "resume-ir.error.v2",
      status: "error",
      error: {
        code: "SERVICE_BLOCKED",
        action: "repair_required",
        capability: null,
        reason: "runtime_invariant",
      },
    },
    status: 503,
  }));
  await assert.rejects(
    submitImportExpectedRejected(wrong, {
      roots: ["/private/synthetic"],
      profile: "explicit",
      max_files: 1,
    }),
    /optional_runtime_claim_gate_invalid/,
  );
});

test("OCR backlog admission requires the exact bounded import receipt", async (context) => {
  const connection = await startServer(context, () => ({
    body: {
      schema_version: "daemon.import.v1",
      status: "accepted",
      accepted_roots: 1,
      new_tasks: 1,
      task_ids: [`imp_${"d".repeat(32)}`],
      scan_profile: "explicit",
      scan_file_limit: 1,
    },
    status: 202,
  }));
  await submitOcrBacklogImport(connection, {
    roots: ["/private/synthetic"],
    profile: "explicit",
    max_files: 1,
  });

  const malformed = await startServer(context, () => ({
    body: {
      schema_version: "daemon.import.v1",
      status: "accepted",
      accepted_roots: 1,
      new_tasks: 1,
      task_ids: [`imp_${"d".repeat(32)}`],
      scan_profile: "explicit",
      scan_file_limit: 1,
      private_debug: true,
    },
    status: 202,
  }));
  await assert.rejects(
    submitOcrBacklogImport(malformed, {
      roots: ["/private/synthetic"],
      profile: "explicit",
      max_files: 1,
    }),
    /optional_runtime_claim_gate_invalid/,
  );
});

test("OCR fault fixture is a private copy of the committed synthetic scan", async (context) => {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-ocr-fault-fixture-")),
  );
  await chmod(root, 0o700);
  context.after(() => rm(root, { force: true, recursive: true }));
  const fixture = await createOcrFaultFixture({ root }, REPO_ROOT);
  assert.deepEqual(fixture.request, {
    roots: [fixture.root],
    profile: "explicit",
    max_files: 1,
  });
  const destination = path.join(fixture.root, "synthetic-scanned-resume.pdf");
  const [actual, expected, metadata] = await Promise.all([
    readFile(destination),
    readFile(path.join(REPO_ROOT, "tests/fixtures/resumes/synthetic-scanned-resume.pdf")),
    lstat(destination),
  ]);
  assert.deepEqual(actual, expected);
  assert.equal(metadata.mode & 0o777, 0o600);
  assert.equal(metadata.nlink, 1);
});

test("classifier epoch reader is exact, read-only, and closed-environment", async (context) => {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-classifier-epoch-")),
  );
  await chmod(root, 0o700);
  context.after(() => rm(root, { force: true, recursive: true }));
  const digest = "e".repeat(64);
  const fileName = `metadata-v29-${digest.slice(0, 16)}.sqlite3`;
  await writeFile(
    path.join(root, "metadata-active.v1"),
    `resume-ir.metadata-active.v1\nfile=${fileName}\nschema=29\ndigest=${digest}\n`,
    { mode: 0o600 },
  );
  const invocations = [];
  const epoch = await readActiveClassifierEpoch(root, async (...args) => {
    invocations.push(args);
    return {
      overflow: false,
      status: 0,
      stderr: "",
      stdout: "precision_first_v4\n",
      timedOut: false,
    };
  });
  assert.equal(epoch, "precision_first_v4");
  assert.equal(invocations.length, 1);
  assert.equal(invocations[0][0], "/usr/bin/sqlite3");
  assert.deepEqual(invocations[0][1].slice(0, 3), [
    "-batch",
    "-readonly",
    "-noheader",
  ]);
  assert.equal(invocations[0][1][3], path.join(root, fileName));
  assert.deepEqual(invocations[0][2].env, CLOSED_SYSTEM_TOOL_ENV);

  await assert.rejects(
    readActiveClassifierEpoch(root, async () => ({
      overflow: false,
      status: 0,
      stderr: "",
      stdout: "precision_first_v4\nsecond_epoch\n",
      timedOut: false,
    })),
    /optional_runtime_classifier_epoch_invalid/,
  );
});
