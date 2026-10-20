/* Actual stage observations, separate from key-animation events. */
const DIAG_LABELS = {
  parse: "形式の読み取り",
  session_load: "セッションの取得・準備",
  prekey_process: "事前鍵と相手の確認",
  session_candidates: "セッション候補での復号",
  ratchet_prepare: "候補の暗号処理を準備",
  key_prepare: "メッセージ用の鍵を準備",
  authentication: "認証の検証（MAC・アドレス）",
  plaintext: "本文の復号",
  identity_trust: "相手の身元の鍵を信頼できるか確認",
  identity_save: "相手の身元の鍵を保存",
  prekey_cleanup: "使用した事前鍵の後処理",
  session_save: "セッションの保存",
  sender_key_load: "送信者の Sender Key 記録を取得",
  sender_key_state: "メッセージに対応する鍵の状態を選択",
  signature: "送信者の署名を検証",
  sender_key_save: "Sender Key 記録を保存"
};
const DIAG_SHORT = {
  parse: "形式", session_load: "セッション取得", prekey_process: "事前鍵の処理",
  session_candidates: "候補で復号", ratchet_prepare: "復号の準備", key_prepare: "本文の鍵を準備",
  authentication: "MAC・アドレス", plaintext: "本文を復号", identity_trust: "相手の信頼確認",
  identity_save: "相手の鍵を保存", prekey_cleanup: "事前鍵の後処理", session_save: "セッション保存",
  sender_key_load: "送信者の鍵を取得", sender_key_state: "対応する鍵を選択", signature: "署名の検証",
  sender_key_save: "送信者の鍵を保存"
};
const DIAG_STATUS = { ok: "成功", error: "失敗", started: "開始", skipped: "未実行", unknown: "記録なし" };
const DIAG_SYMBOL = { ok: "✓", error: "✕", started: "…", skipped: "—", unknown: "?" };
const DIAG_GROUP_ORDER = ["parse", "sender_key_load", "sender_key_state", "signature", "key_prepare", "plaintext", "sender_key_save"];

function diagnosisRows(events, order) {
  const latest = new Map();
  events.forEach((event) => latest.set(event.stage, event));
  const failedAt = order.findIndex((stage) => latest.get(stage)?.status === "error");
  return order.map((stage, index) => {
    const event = latest.get(stage);
    return {
      stage,
      label: DIAG_LABELS[stage],
      status: event ? (Object.hasOwn(DIAG_STATUS, event.status) ? event.status : "unknown") : (failedAt >= 0 && index > failedAt ? "skipped" : "unknown"),
      detail: event?.detail || "",
      source: event?.source || ""
    };
  });
}

function receiveDiagnosis(action, message, options = {}) {
  if (!action) return null;
  const group = options.mode === "group";
  const stages = (action.events || []).filter((event) => event.kind === "decrypt_stage");
  if ((group && !stages.length && !["deliver", "tamper", "replay"].includes(action.type)) ||
      (!group && action.type === "send" && !action.delivered_now)) return null;
  if (!stages.length) return { recorded: false };
  const prekey = !group && message?.kind === "prekey";
  const order = group ? [...DIAG_GROUP_ORDER] : ["parse", "session_load"];
  if (!group) {
    if (prekey) order.push("prekey_process");
    order.push("session_candidates");
    if (!prekey) order.push("identity_trust");
    order.push("identity_save");
    if (prekey) order.push("prekey_cleanup");
    order.push("session_save");
  }
  const overall = stages.filter((event) => event.candidate === undefined || event.candidate === null);
  const candidates = new Map();
  if (!group) stages.forEach((event) => {
    if (event.candidate === undefined || event.candidate === null) return;
    if (!candidates.has(event.candidate)) candidates.set(event.candidate, []);
    candidates.get(event.candidate).push(event);
  });
  return {
    recorded: true,
    messageId: action.message_id,
    group, prekey,
    sender: options.senderLabel || message?.from || "",
    receiver: options.receiverLabel || (group ? action.who : message?.to) || "",
    ok: action.outcome === "ok",
    error: action.error || "",
    rows: diagnosisRows(overall, order),
    candidates: [...candidates.entries()].map(([id, events]) => ({
      id,
      kind: events[0].candidate_kind,
      result: events.filter((event) => event.stage === "candidate").at(-1)?.status || "unknown",
      rows: diagnosisRows(events, ["ratchet_prepare", "key_prepare", "authentication", "plaintext"])
    }))
  };
}

function diagnosisHtml(action, message, options = {}) {
  const model = receiveDiagnosis(action, message, options);
  if (!model) return "";
  const escape = (value) => String(value).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
  if (!model.recorded) return '<h3>受信の診断</h3><p class="hint">この操作には段階の記録がありません。</p>';
  const candidateName = (candidate) => "候補 " + (candidate.id + 1) + "（" + (candidate.kind === "current" ? "現在" : candidate.kind === "previous" ? "過去" : "種類の記録なし") + "）";
  const selected = model.candidates.findLast((candidate) => candidate.result === "ok") || model.candidates.at(-1);
  // Inline one actual candidate at the point it was tried; all trials remain in the details.
  const pipeline = model.rows.flatMap((item) => item.stage === "session_candidates" && selected ? selected.rows : [item]);
  const failure = model.ok ? null : (pipeline.find((item) => item.status === "error") || model.rows.find((item) => item.status === "error"));
  const trialFailures = model.candidates.filter((candidate) => candidate.result === "error").length;
  const route = model.sender && model.receiver ? escape(model.sender) + ' → ' + escape(model.receiver) : "";
  const kind = model.group ? "グループ · Sender Keys" : model.prekey ? "1対1 · 初回の鍵交換付き" : "1対1 · 通常メッセージ";
  const chip = (item, index) => '<li class="diag-chip diag-' + item.status + '" title="' + escape(item.label + "：" + DIAG_STATUS[item.status]) + '"><span class="diag-chip-name"><span class="diag-number">' + (index + 1) + '</span>' + escape(DIAG_SHORT[item.stage] || item.label) + '</span><span class="diag-chip-state"><span aria-hidden="true">' + DIAG_SYMBOL[item.status] + '</span> ' + DIAG_STATUS[item.status] + '</span></li>';
  const detailRow = (item, nested = "") => '<li class="diag-record diag-' + item.status + '"><div><span>' + escape(item.label) + '</span><strong>' + DIAG_STATUS[item.status] + '</strong></div>' +
    (item.source ? '<code>' + escape(item.source) + '</code>' : '') +
    (item.detail ? '<pre class="diag-detail">' + escape(item.detail) + '</pre>' : '') + nested + '</li>';
  const candidateDetails = model.candidates.map((candidate) => '<section class="diag-trial"><h4>' + escape(candidateName(candidate)) + '<span class="diag-badge diag-' + candidate.result + '">' + DIAG_STATUS[candidate.result] + '</span></h4><ol>' + candidate.rows.map((item) => detailRow(item)).join("") + '</ol></section>').join("");
  const details = model.rows.map((item) => detailRow(item, item.stage === "session_candidates" ? candidateDetails : "")).join("");
  const trialNote = selected ? '<p class="diag-note">' + (model.candidates.length > 1 ? model.candidates.length + '候補を試行（失敗 ' + trialFailures + '件）。' : '') + escape(candidateName(selected)) + 'の処理を表示。全候補は詳細へ。</p>' : '';
  return '<div class="diag-heading"><h3>受信の診断 <span class="mono">#' + escape(model.messageId) + '</span></h3><strong class="diag-result diag-' + (model.ok ? 'ok' : 'error') + '">' + (model.ok ? '✓ 受信成功' : '✕ 受信失敗') + '</strong></div>' +
    '<p class="diag-context">' + (route ? '<b>' + route + '</b><span> · </span>' : '') + kind + '</p>' +
    (failure ? '<p class="diag-stop" role="status">停止した段階：<strong>' + escape(failure.label) + '</strong></p>' : '') +
    '<ol class="diag-pipeline" aria-label="復号処理の順序">' + pipeline.map(chip).join("") + '</ol>' + trialNote +
    '<details class="diag-more"><summary>関数・エラー' + (model.candidates.length > 1 ? '・全候補' : '') + 'の詳細</summary><div class="diag-detail-scroll" tabindex="0" role="region" aria-label="復号の詳しい記録"><ol>' + details + '</ol>' +
    (model.error ? '<h4>呼び出し元に返ったエラー</h4><pre class="diag-detail">' + escape(model.error) + '</pre>' : '') +
    '<p class="diag-note">' + (model.group ? '署名を確かめてから本文の鍵を準備します。' : '候補内の記録は復号の試行です。候補の失敗と、受信全体の失敗は区別します。') + '「未実行」は、この経路で失敗した段階より後の処理です。「記録なし」は、実行結果を確認できない段階です。</p></div></details>';
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { receiveDiagnosis, diagnosisHtml };
}
