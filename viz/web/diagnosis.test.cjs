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

test("truncated message stops at the observed parser without inventing decryption", () => {
  const received = { ...action([event("parse", "error", undefined, { detail: "message too short" })], "too_short"), type: "tamper", tamper_mode: "truncate" };
  const model = receiveDiagnosis(received, { kind: "prekey" });
  assert.equal(status(model, "parse"), "error");
  assert.equal(status(model, "prekey_process"), "skipped");
  assert.equal(model.candidates.length, 0);
  assert.ok(diagnosisHtml(received, { kind: "prekey" }).includes('停止した段階：<strong>形式の読み取り</strong>'));
});

test("missing session is an observed receiver-copy failure before candidate decryption", () => {
  const received = { ...action([event("parse", "ok"), event("session_load", "error")], "no_session"), type: "tamper", tamper_mode: "no_session" };
  const model = receiveDiagnosis(received, { kind: "whisper" });
  assert.equal(status(model, "parse"), "ok");
  assert.equal(status(model, "session_load"), "error");
  assert.equal(status(model, "session_candidates"), "skipped");
  assert.equal(model.candidates.length, 0);
  const html = diagnosisHtml(received, { kind: "whisper" });
  assert.ok(html.includes('停止した段階：<strong>セッションの取得・準備</strong>'));
  assert.ok(html.includes("保管庫のコピー"));
  assert.ok(html.includes("表示中の元の保管庫とメッセージは変更しません"));
});

test("identity rejection preserves successful plaintext observation and skips persistence", () => {
  const received = { ...action([
    event("parse", "ok"), event("session_load", "ok"),
    event("ratchet_prepare", "ok", 0), event("key_prepare", "ok", 0),
    event("authentication", "ok", 0), event("plaintext", "ok", 0), event("candidate", "ok", 0),
    event("session_candidates", "ok"), event("identity_trust", "error")
  ], "untrusted_identity"), type: "tamper", tamper_mode: "untrusted_identity" };
  const model = receiveDiagnosis(received, { kind: "whisper" });
  assert.equal(model.ok, false);
  assert.equal(model.candidates[0].result, "ok");
  assert.equal(status(model.candidates[0], "plaintext"), "ok");
  assert.equal(status(model, "identity_trust"), "error");
  assert.equal(status(model, "identity_save"), "skipped");
  assert.equal(status(model, "session_save"), "skipped");
  const html = diagnosisHtml(received, { kind: "whisper" });
  assert.ok(html.includes('停止した段階：<strong>相手の身元の鍵を信頼できるか確認</strong>'));
  assert.ok(html.includes("✕ 受信失敗"));
});

test("experiment modes do not substitute for missing stage evidence", () => {
  const received = { ...action([event("parse", "ok")], "no_session"), type: "tamper", tamper_mode: "no_session" };
  const model = receiveDiagnosis(received, { kind: "whisper" });
  assert.equal(status(model, "session_load"), "unknown");
  assert.ok(!diagnosisHtml(received, { kind: "whisper" }).includes("停止した段階"));
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

const groupOptions = { mode: "group", senderLabel: "Alice", receiverLabel: "Bob" };
const groupMessage = { from: "alice" };
const groupStages = ["parse", "sender_key_load", "sender_key_state", "signature", "key_prepare", "plaintext", "sender_key_save"];

test("group success follows Sender Keys order and names its sender and receiver", () => {
  const received = { ...action(groupStages.map((stage) => event(stage, "ok")), "ok"), who: "bob" };
  const model = receiveDiagnosis(received, groupMessage, groupOptions);
  assert.equal(model.group, true);
  assert.equal(model.ok, true);
  assert.deepEqual(model.rows.map((row) => row.stage), groupStages);
  assert.equal(model.rows.every((row) => row.status === "ok"), true);
  assert.deepEqual(model.candidates, []);
  const html = diagnosisHtml(received, groupMessage, groupOptions);
  assert.ok(html.includes("Alice → Bob"));
  assert.ok(html.includes("グループ · Sender Keys"));
  assert.ok(html.includes("受信成功"));
  assert.ok(!html.includes("MAC・アドレス"));
});

test("group signature rejection occurs before message-key preparation", () => {
  const received = action([
    ...groupStages.slice(0, 3).map((stage) => event(stage, "ok")),
    event("signature", "error", undefined, { source: "group_decrypt", detail: "signature verification failed" })
  ]);
  const model = receiveDiagnosis(received, groupMessage, groupOptions);
  assert.equal(status(model, "signature"), "error");
  assert.equal(status(model, "key_prepare"), "skipped");
  assert.equal(status(model, "plaintext"), "skipped");
  assert.equal(status(model, "sender_key_save"), "skipped");
  const html = diagnosisHtml(received, groupMessage, groupOptions);
  assert.ok(html.includes('停止した段階：<strong>送信者の署名を検証</strong>'));
  assert.ok(html.indexOf('停止した段階') < html.indexOf('<details'));
  assert.ok(html.includes("group_decrypt"));
});

test("group missing sender record skips signature and key operations", () => {
  const model = receiveDiagnosis(action([
    event("parse", "ok"), event("sender_key_load", "error")
  ]), groupMessage, groupOptions);
  assert.equal(status(model, "sender_key_load"), "error");
  assert.equal(status(model, "sender_key_state"), "skipped");
  assert.equal(status(model, "signature"), "skipped");
  assert.equal(status(model, "key_prepare"), "skipped");
});

test("group duplicate can pass signature but fail while obtaining the used message key", () => {
  const received = action([
    ...groupStages.slice(0, 4).map((stage) => event(stage, "ok")),
    event("key_prepare", "error", undefined, { detail: "message with old counter" })
  ], "duplicate");
  const model = receiveDiagnosis(received, groupMessage, groupOptions);
  assert.equal(status(model, "signature"), "ok");
  assert.equal(status(model, "key_prepare"), "error");
  assert.equal(status(model, "plaintext"), "skipped");
  assert.equal(status(model, "sender_key_save"), "skipped");
});

test("group sends and joins without decrypt records do not invent a receive", () => {
  ["send", "join"].forEach((type) => {
    const operation = { type, events: [{ kind: "sender_key_created" }] };
    assert.equal(receiveDiagnosis(operation, groupMessage, groupOptions), null);
    assert.equal(diagnosisHtml(operation, groupMessage, groupOptions), "");
  });
  assert.equal(receiveDiagnosis(action([]), groupMessage, groupOptions).recorded, false);
  const model = receiveDiagnosis(action([event("parse", "ok")], "ok"), groupMessage, groupOptions);
  assert.equal(status(model, "sender_key_save"), "unknown");
});

test("compact fallback summary preserves failed trials without claiming the receive stopped", () => {
  const received = action([
    event("parse", "ok"), event("session_load", "ok"),
    event("authentication", "error", 0), event("candidate", "error", 0),
    event("authentication", "ok", 1), event("plaintext", "ok", 1), event("candidate", "ok", 1),
    event("session_candidates", "ok"), event("identity_trust", "ok"),
    event("identity_save", "ok"), event("session_save", "ok")
  ], "ok");
  const html = diagnosisHtml(received, { kind: "whisper" });
  assert.ok(html.includes("受信成功"));
  assert.ok(html.includes("2候補を試行（失敗 1件）"));
  assert.ok(!html.includes("停止した段階"));
  assert.equal((html.match(/<details/g) || []).length, 1);
  assert.ok(html.includes("候補 1（現在）"));
  assert.ok(html.includes("候補 2（過去）"));
});

test("group route and API errors cannot introduce markup", () => {
  const received = { ...action([event("parse", "error")]), error: '<a href="x">bad</a>' };
  const html = diagnosisHtml(received, groupMessage, { mode: "group", senderLabel: "<script>", receiverLabel: "<img>" });
  assert.ok(html.includes("&lt;script&gt; → &lt;img&gt;"));
  assert.ok(html.includes("&lt;a href=&quot;x&quot;&gt;bad&lt;/a&gt;"));
  assert.ok(!html.includes("<script>"));
  assert.ok(!html.includes("<img>"));
});
