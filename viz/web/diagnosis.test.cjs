const { test } = require("node:test");
const assert = require("node:assert/strict");
const { receiveDiagnosis, diagnosisHtml } = require("./diagnosis.js");

const event = (stage, status, candidate, extra = {}) => ({
  kind: "decrypt_stage", stage, status,
  ...(candidate === undefined ? {} : { candidate, candidate_kind: candidate === 0 ? "current" : "previous" }),
  ...extra
});
const action = (events, outcome = "error") => ({ type: "deliver", message_id: 3, outcome, events });
const status = (model, stage) => model.rows.find((row) => row.stage === stage)?.status;

test("MAC rejection leaves plaintext and persistence unexecuted", () => {
  const model = receiveDiagnosis(action([
    event("parse", "ok"), event("session_load", "ok"),
    event("candidate", "started", 0), event("ratchet_prepare", "ok", 0),
    event("key_prepare", "ok", 0), event("authentication", "error", 0),
    event("candidate", "error", 0), event("session_candidates", "error"),
    event("receive", "error")
  ]), { kind: "whisper" });
  assert.equal(model.ok, false);
  assert.equal(status(model, "session_save"), "skipped");
  assert.equal(status(model.candidates[0], "plaintext"), "skipped");
});

test("a failed current candidate does not make archived success a receive failure", () => {
  const model = receiveDiagnosis(action([
    event("parse", "ok"), event("session_load", "ok"),
    event("authentication", "error", 0), event("candidate", "error", 0),
    event("authentication", "ok", 1), event("plaintext", "ok", 1), event("candidate", "ok", 1),
    event("session_candidates", "ok"), event("identity_trust", "ok"),
    event("identity_save", "ok"), event("session_save", "ok"), event("receive", "ok")
  ], "ok"), { kind: "whisper" });
  assert.equal(model.ok, true);
  assert.equal(model.candidates.length, 2);
  assert.equal(model.candidates[0].result, "error");
  assert.equal(model.candidates[1].result, "ok");
  assert.equal(status(model.candidates[0], "plaintext"), "skipped");
  assert.equal(status(model.candidates[1], "plaintext"), "ok");
  assert.equal(status(model, "session_save"), "ok");
});

test("PreKey has its own preparation and cleanup, without an invented later trust check", () => {
  const model = receiveDiagnosis(action([
    event("parse", "ok"), event("session_load", "ok"), event("prekey_process", "error")
  ]), { kind: "prekey" });
  assert.equal(status(model, "prekey_process"), "error");
  assert.equal(status(model, "prekey_cleanup"), "skipped");
  assert.equal(status(model, "identity_trust"), undefined);
  assert.equal(model.candidates.length, 0);
});

test("parse failure is distinguished from a failure inside decryption", () => {
  const model = receiveDiagnosis(action([event("parse", "error")]), { kind: "whisper" });
  assert.equal(status(model, "parse"), "error");
  assert.equal(status(model, "session_load"), "skipped");
  assert.equal(model.candidates.length, 0);
});

test("missing observations are not presented as successful or unexecuted", () => {
  assert.equal(receiveDiagnosis(null, null), null);
  assert.equal(receiveDiagnosis({ type: "send", delivered_now: false }, null), null);
  assert.equal(receiveDiagnosis(action([]), null).recorded, false);
  const model = receiveDiagnosis(action([event("parse", "ok")], "ok"), { kind: "whisper" });
  assert.equal(status(model, "session_save"), "unknown");
});

test("error and source strings are rendered as text", () => {
  const html = diagnosisHtml(action([
    event("parse", "error", undefined, { detail: "<img src=x>", source: "<script>" })
  ]), { kind: "whisper" });
  assert.ok(html.includes("&lt;img src=x&gt;"));
  assert.ok(html.includes("&lt;script&gt;"));
  assert.ok(!html.includes("<img"));
});
