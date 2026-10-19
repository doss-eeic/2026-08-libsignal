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
  session_save: "セッションの保存"
};
const DIAG_STATUS = { ok: "成功", error: "失敗", started: "開始", skipped: "未実行", unknown: "記録なし" };

function diagnosisRows(events, order) {
  const latest = new Map();
  events.forEach((event) => latest.set(event.stage, event));
  const failedAt = order.findIndex((stage) => latest.get(stage)?.status === "error");
  return order.map((stage, index) => {
    const event = latest.get(stage);
    return {
      stage,
      label: DIAG_LABELS[stage],
      status: event?.status || (failedAt >= 0 && index > failedAt ? "skipped" : "unknown"),
      detail: event?.detail || "",
      source: event?.source || ""
    };
  });
}

function receiveDiagnosis(action, message) {
  if (!action || (action.type === "send" && !action.delivered_now)) return null;
  const stages = (action.events || []).filter((event) => event.kind === "decrypt_stage");
  if (!stages.length) return { recorded: false };
  const prekey = message?.kind === "prekey";
  const order = ["parse", "session_load"];
  if (prekey) order.push("prekey_process");
  order.push("session_candidates");
  if (!prekey) order.push("identity_trust");
  order.push("identity_save");
  if (prekey) order.push("prekey_cleanup");
  order.push("session_save");

  const overall = stages.filter((event) => event.candidate === undefined || event.candidate === null);
  const candidates = new Map();
  stages.forEach((event) => {
    if (event.candidate === undefined || event.candidate === null) return;
    if (!candidates.has(event.candidate)) candidates.set(event.candidate, []);
    candidates.get(event.candidate).push(event);
  });
  return {
    recorded: true,
    messageId: action.message_id,
    prekey,
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

function diagnosisHtml(action, message) {
  const model = receiveDiagnosis(action, message);
  if (!model) return "";
  const escape = (value) => String(value).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
  if (!model.recorded) return '<h3>受信の診断</h3><p class="hint">この操作には段階の記録がありません。</p>';
  const row = (item, nested = "") => '<li class="diag-row diag-' + item.status + '"><div><span>' + escape(item.label) + '</span><strong>' + DIAG_STATUS[item.status] + '</strong></div>' +
    (item.detail ? '<p class="diag-detail">' + escape(item.detail) + '</p>' : "") +
    (item.source ? '<details class="diag-source"><summary>記録した関数</summary><code>' + escape(item.source) + '</code></details>' : "") + nested + '</li>';
  const candidateHtml = model.candidates.map((candidate) => '<div class="diag-candidate"><h4>候補 ' + (candidate.id + 1) + '：' + (candidate.kind === "current" ? "現在のセッション" : "過去のセッション") +
    '<span class="diag-badge diag-' + candidate.result + '">' + DIAG_STATUS[candidate.result] + '</span></h4><ol>' + candidate.rows.map((item) => row(item)).join("") + '</ol></div>').join("");
  const rows = model.rows.map((item) => row(item, item.stage === "session_candidates" ? candidateHtml : "")).join("");
  return '<h3>受信の診断 <span class="mono">#' + model.messageId + '</span></h3><p class="hint">' + (model.prekey ? "初回の鍵交換情報付きメッセージ" : "通常の1対1メッセージ") + '</p>' +
    '<ol class="diag-stages">' + rows + '</ol>' +
    '<p class="diag-result ' + (model.ok ? "diag-ok" : "diag-error") + '">受信全体：' + (model.ok ? "成功" : "失敗") + '</p>' +
    (model.error ? '<details class="diag-final-error"><summary>呼び出し元に返ったエラー</summary><pre>' + escape(model.error) + '</pre></details>' : "") +
    '<p class="hint">候補内の鍵の動きは試行中の記録です。候補の失敗と、受信全体の失敗は分けて表示しています。未実行は、この経路で失敗した段階より後の処理を表します。</p>';
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { receiveDiagnosis, diagnosisHtml };
}
