"use strict";
const SCENES = window.SCENES;
const CHAT = window.CHAT;

const CHARS = {
  alice: { name: "Alice", initial: "A", role: "" },
  bob: { name: "Bob", initial: "B", role: "" },
  carol: { name: "Carol", initial: "C", role: "" },
  dave: { name: "Dave", initial: "D", role: "" },
  mallory: { name: "Mallory", initial: "M", role: "通信路の攻撃者" }
};
const REASONS = { duplicate: "重複", bad_mac: "MAC不一致", bad_signature: "署名不一致" };
const SCENE_ORDER = ["one-to-one", "group", "attacks", "live"];

let sceneId = "one-to-one";
let stepIdx = 0;

const $ = (id) => document.getElementById(id);
function esc(s) {
  return String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
}
function hueOf(fp) { return Math.round((parseInt(fp.slice(0, 2), 16) / 255) * 360); }
function chipStyle(fp) { return "background:hsl(" + hueOf(fp) + " 62% var(--chip-l))"; }

function avatarFallback(img) {
  const id = img.dataset.char;
  const d = document.createElement("div");
  d.className = "avatar dot";
  d.style.background = "var(--c-" + id + ")";
  d.textContent = CHARS[id].initial;
  d.setAttribute("aria-hidden", "true");
  img.replaceWith(d);
}

function whoHtml(id, dashed) {
  return '<span class="who' + (dashed ? " dashed" : "") + '"><i style="background:var(--c-' + id + ')"></i><span class="dot">' + CHARS[id].name + "</span></span>";
}

function keyHtml(e) {
  const unit = e.unit || "index";
  let tag = "";
  if (e.old) tag = '<span class="tag muted">' + esc(e.tag || "置き換え済み") + "</span>";
  else if (e.changed) tag = '<span class="tag">' + esc(e.tag || "更新") + "</span>";
  const box =
    '<div class="key' + (e.old ? " old" : "") + (e.changed && !e.old ? " changed" : "") + '" data-kid="' + esc(e.id) + '">' +
    '<div class="key-top"><span class="label">' + esc(e.label) + "</span>" + tag + "</div>" +
    ((e.id === "root" || e.noindex || e.index === undefined) ? "" : '<div class="idx mono">' + unit + " " + e.index + "</div>") +
    '<div class="fp mono"><span class="chipc" style="' + chipStyle(e.fp) + '"></span><span class="fp-text">fp ' + e.fp + "</span></div></div>";
  if (e.replacedFrom) {
    const old =
      '<div class="key old"><div class="key-top"><span class="label">' + esc(e.label) + '</span><span class="tag muted">置き換え前</span></div>' +
      '<div class="fp mono"><span class="chipc" style="' + chipStyle(e.replacedFrom) + '"></span><span class="fp-text">fp ' + e.replacedFrom + "</span></div></div>";
    return old + box;
  }
  return box;
}

function portraitHtml(id, cls) {
  return '<div class="portrait ' + (cls || "") + '"><img src="img/' + id + '.png" alt="" data-char="' + id + '" onerror="avatarFallback(this)"></div>';
}

function chipHtml(c, cur) {
  let st = null;
  c.tl.forEach((t) => { if (t[0] <= cur) st = t; });
  if (!st) return "";
  const name = CHARS[c.to].name;
  const extra = st[2] ? esc(st[2]) : "";
  let h = "";
  if (st[1] === "pend") h = '<span class="chip pend">' + name + " 送信中</span>";
  else if (st[1] === "ok") {
    h = '<span class="chip ok">&#10003; ' + name + " 読めた</span>";
    if (extra) h += '<span class="chip order">' + extra + "</span>";
  } else h = '<span class="chip bad">' + name + " " + (c.verb || "読めない") + "：" + extra + "</span>";
  return h;
}

function renderChat(sceneKey, cur) {
  const chat = CHAT[sceneKey];
  $("chat-title").textContent = chat.title;
  let h = "";
  chat.items.forEach((it) => {
    if (it.at > cur) return;
    const isCur = it.at === cur;
    const chips = (it.chips || []).map((c) => chipHtml(c, cur)).join("");
    if (it.kind === "sys") {
      h += '<div class="sys' + (isCur ? " cur" : "") + '" data-cur="' + (isCur ? 1 : 0) + '">' + esc(it.text) +
        (chips ? '<div class="chips">' + chips + "</div>" : "") + "</div>";
    } else {
      h += '<div class="chat-item' + (isCur ? " cur" : "") + (it.mal ? " mal" : "") + '" data-cur="' + (isCur ? 1 : 0) + '">' + portraitHtml(it.who, "p32") +
        '<div class="chat-main"><div class="chat-name"><span class="dot">' + CHARS[it.who].name + "</span>" +
        (it.mal ? '<span class="tag bad">割り込み</span>' : "") + "</div>" +
        '<div class="bubble">' + esc(it.text) + "</div>" +
        (it.meta ? '<div class="meta mono">' + esc(it.meta) + "</div>" : "") +
        (chips ? '<div class="chips">' + chips + "</div>" : "") + "</div></div>";
    }
  });
  const log = $("chat-log");
  log.innerHTML = h;
  scrollToCur(log);
}

// チャット枠の中だけをスクロールする（ページは動かさない）
function scrollToCur(log) {
  const els = log.querySelectorAll('[data-cur="1"]');
  if (!els.length) { log.scrollTop = 0; return; }
  const first = els[0], last = els[els.length - 1];
  const bottom = last.offsetTop + last.offsetHeight + 4;
  let top = log.scrollTop;
  if (bottom > top + log.clientHeight) top = bottom - log.clientHeight;
  if (first.offsetTop < top) top = Math.max(0, first.offsetTop - 4);
  const reduce = window.matchMedia && matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (log.scrollTo && !reduce) log.scrollTo({ top: top, behavior: "smooth" });
  else log.scrollTop = top;
}

function renderStage(scene, step) {
  const stage = $("stage");
  const cols = scene.cast.length === 4 ? "repeat(2, minmax(0, 1fr))" :
    scene.cast.indexOf("mallory") >= 0 ? "minmax(0, 1fr) minmax(0, 0.7fr) minmax(0, 1fr)" :
    "repeat(" + scene.cast.length + ", minmax(0, 1fr))";
  stage.style.setProperty("--cols", cols);
  const m = step.message;
  let h = "";
  scene.cast.forEach((id) => {
    const c = CHARS[id];
    const dim = (step.dim || []).indexOf(id) >= 0;
    const alert = (step.alert || []).indexOf(id) >= 0;
    const bad = step.result !== "ok";
    let tcls = "";
    if (m && !m.inflight && m.to.indexOf(id) >= 0) tcls = bad ? " t-bad" : " t-ok";
    h += '<div class="col' + (dim ? " dim" : "") + tcls + '" data-char="' + id + '">';
    h += '<div class="col-head">' + portraitHtml(id) +
      '<div><div class="cname dot">' + c.name + "</div>" + (c.role ? '<div class="crole">' + c.role + "</div>" : "") + "</div></div>";
    if (id === "mallory") {
      h += '<div class="role-note">鍵は持っていない。流れてくる暗号文を書き換えたり再送したりできる。</div>';
    }
    const keys = step.state[id] || [];
    if (keys.length) h += '<div class="keys">' + keys.map(keyHtml).join("") + "</div>";
    else if (id !== "mallory") h += '<div class="keys"></div>';
    const note = step.notes && step.notes[id];
    if (note) h += '<p class="col-note' + (alert ? " alert" : "") + '">' + esc(note) + "</p>";
    h += "</div>";
  });
  stage.innerHTML = h;
}

/* イベント → やさしい日本語（サンプルと libsignal の実出力の両方に使う） */
function evText(e, sceneKey) {
  const N = (id) => (CHARS[id] ? CHARS[id].name : id);
  const att = sceneKey === "attacks";
  const live = sceneKey === "live";
  const msgLabel = (i) => (att ? "#" + (i + 1) : (live ? "counter " : "index ") + i + " のメッセージ");
  const chain = (c) => (c === "recv" ? "受信" : "送信");
  switch (e.kind) {
    case "session_created":
      return N(e.who) + " のセッションができた（最初の鍵を作った）";
    case "root_step":
      if (e.source) return N(e.who) + " のルート鍵ができた（最初の鍵交換 " + e.source.toUpperCase() + "）";
      if (e.from) return N(e.who) + " のルート鍵を新しくした（" + e.from.slice(0, 4) + "… → " + e.to.slice(0, 4) + "…）";
      if (e.old_fp) return N(e.who) + " のルート鍵を新しくした（" + e.old_fp.slice(0, 4) + "… → " + (e.fp || "").slice(0, 4) + "…）";
      if (e.fp) return N(e.who) + " のルート鍵ができた（fp " + e.fp + "）";
      return N(e.who) + " が、新しいルート鍵から" + chain(e.chain) + "の鎖の元になる鍵を取り出した";
    case "chain_step":
      if (e.from === null || e.from === undefined) return N(e.who) + " の" + chain(e.chain) + "の鎖を index " + e.to + " で作った";
      return N(e.who) + " の" + chain(e.chain) + "の鎖が " + e.from + " → " + e.to + " に進んだ";
    case "message_key":
      return N(e.who) + " が、このメッセージ専用の鍵を作った（index " + e.index + "、使ったら捨てる）";
    case "dh_ratchet":
      if (e.step === "recv") return N(e.who) + " は、相手の返信を機に新しいDH鍵で鎖を作り直した";
      if (e.step === "send") return N(e.who) + " は、返信を機に、新しいDH鍵で鎖を作り直した";
      return N(e.who) + " は、新しいDH鍵で鎖を作り直した" + (e.old_fp ? "（" + e.old_fp.slice(0, 4) + "… → " + (e.fp || "").slice(0, 4) + "…）" : "");
    case "skipped_key_stored":
      return N(e.who) + " が、まだ届いていない " + msgLabel(e.index) + "用の鍵を取り置いた";
    case "skipped_key_used":
      return N(e.who) + " が、取り置いた鍵で " + msgLabel(e.index) + "を開いて、鍵を捨てた";
    case "duplicate_rejected":
      if (e.iteration !== undefined) return N(e.who) + " は iteration " + e.iteration + " の鍵を持っていない（今は " + e.current + "）ので、開けなかった";
      return N(e.who) + " は、使い済みの番号（" + msgLabel(e.index) + "）なので弾いた";
    case "mac_rejected":
      return N(e.who) + " は、MAC（改ざん検知の印）が合わないので弾いた" + (e.expected ? "（期待 " + e.expected + " / 実際 " + e.got + "）" : "");
    case "sender_key_distributed":
      return N(e.from) + " が " + e.to.map(N).join(" と ") + " に Sender Key（iteration " + e.iteration + "）を1対1の暗号で配った";
    case "sender_key_received":
      return N(e.who) + " が " + N(e.from) + " の Sender Key（iteration " + e.iteration + "）を受け取った";
    case "sender_chain_step":
      if (e.from === undefined) return N(e.who) + " の Sender Key の鎖を iteration " + e.iteration + " で作った";
      return N(e.who) + " の Sender Key の鎖が " + e.from + " → " + e.to + " に進んだ";
    case "signature_checked":
      return e.ok ? N(e.who) + " が送信者の署名を確かめた：一致した" : N(e.who) + " が送信者の署名を確かめた：一致しなかったので弾いた";
    default:
      return JSON.stringify(e);
  }
}

function renderHappen(step, sceneKey) {
  $("happen-list").innerHTML = step.events.map((e) =>
    "<li>" + esc(evText(e, sceneKey)) + '<span class="ev-tag">' + esc(e.kind) + "</span></li>").join("");
}

function render() {
  const scene = SCENES[sceneId];
  const isLive = sceneId === "live";
  $("composer").hidden = !isLive;
  $("btn-reset").hidden = !isLive;
  $("live-note").hidden = !isLive;
  $("controls").hidden = isLive;
  $("receive-diagnosis").hidden = !isLive;
  if (isLive) {
    $("live-error").hidden = true;
    SCENE_ORDER.forEach((id) => {
      const b = $("tab-" + id);
      b.setAttribute("aria-selected", id === sceneId ? "true" : "false");
      b.tabIndex = id === sceneId ? 0 : -1;
    });
    $("scene-title").textContent = scene.title;
    renderLive();
    return;
  }
  $("live-error").hidden = true;
  $("test-panel").hidden = true;
  $("panel-main").hidden = false;
  $("caption").hidden = false;
  $("json-details").hidden = false;
  const step = scene.steps[stepIdx];
  const total = scene.steps.length;

  SCENE_ORDER.forEach((id) => {
    const b = $("tab-" + id);
    b.setAttribute("aria-selected", id === sceneId ? "true" : "false");
    b.tabIndex = id === sceneId ? 0 : -1;
  });
  $("scene-title").textContent = scene.title;

  renderChat(sceneId, stepIdx);
  renderStage(scene, step);
  renderHappen(step, sceneId);

  $("cap-title").textContent = step.caption;
  $("cap-text").textContent = step.explain;
  $("step-counter").textContent = stepIdx + 1 + " / " + total;
  $("btn-prev").disabled = stepIdx === 0;
  $("btn-next").disabled = stepIdx === total - 1;

  const dots = $("dots");
  dots.innerHTML = "";
  scene.steps.forEach((s, i) => {
    const b = document.createElement("button");
    b.type = "button";
    b.id = "dot-" + i;
    b.setAttribute("aria-label", "ステップ " + (i + 1) + "：" + s.action);
    if (i === stepIdx) b.setAttribute("aria-current", "step");
    b.className = i < stepIdx ? "done" : i === stepIdx ? "cur" : "";
    b.innerHTML = "<span></span>";
    b.addEventListener("click", () => go(i));
    dots.appendChild(b);
  });

  $("json-pre").textContent = JSON.stringify(step.events, null, 2);
}

function go(i) {
  const total = SCENES[sceneId].steps.length;
  stepIdx = Math.max(0, Math.min(total - 1, i));
  render();
}
function setScene(id) {
  sceneId = id;
  stepIdx = 0;
  render();
  if (id === "live") enterLive();
}

/* ================= ④ libsignal で送る（手元の Rust サーバーが返した実際の鍵と出来事を表示） ================= */
SCENES.live = { tab: "④ libsignal で送る", title: "自分で送って、本物の libsignal の鍵の動きを見る", cast: ["alice", "mallory", "bob"], steps: [] };

let ST = null;          // 最新の State（サーバーが返したもの）
let PREV = null;        // ひとつ前の State（変化の強調表示に使う）
let LOAD = "idle";      // idle / loading / ready / down
let LBUSY = false;
let LMSG = "";
let SENDER = "alice";
let DELIVER_NOW = true;
let GEN = null;         // 書き出し済みのテスト {name, path, code, stale}
let RUNNING = false;
let TESTS = [];
let runTimer = null;
const OUT_JA = { ok: "読めた", duplicate: "重複", bad_mac: "MAC不一致", error: "エラー" };
const NAME_RE = /^[a-z][a-z0-9_]{0,39}$/;
const OTHER = { alice: "bob", bob: "alice" };
const nm = (id) => CHARS[id].name;

async function api(method, path, body) {
  let res;
  try {
    res = await fetch(path, {
      method,
      headers: body !== undefined ? { "Content-Type": "application/json" } : undefined,
      body: body !== undefined ? JSON.stringify(body) : undefined
    });
  } catch (e) {
    const er = new Error("down");
    er.down = true;
    throw er;
  }
  let data = null;
  try { data = await res.json(); } catch (e) { /* 本文なし */ }
  if (!res.ok) {
    const er = new Error((data && data.error) || "サーバーがエラーを返しました（" + res.status + "）");
    er.status = res.status;
    throw er;
  }
  return data;
}

function setState(next, keepPrev) {
  if (!keepPrev) PREV = ST;
  ST = next;
  if (GEN) GEN.stale = true;
}

async function enterLive() {
  LOAD = "loading";
  LMSG = "";
  renderLive();
  try {
    const st = await api("GET", "/api/state");
    ST = st; PREV = null; GEN = null;
    LOAD = "ready";
    await loadTests();
  } catch (e) {
    if (LOAD !== "ready") LOAD = "down";
  }
  if (sceneId === "live") renderLive();
}
async function loadTests() {
  try { TESTS = (await api("GET", "/api/tests")).tests || []; } catch (e) { /* 一覧が取れなくても続ける */ }
}
async function runLive(fn) {
  if (LBUSY || LOAD !== "ready") return;
  LBUSY = true;
  LMSG = "";
  renderLive();
  try { await fn(); } catch (e) {
    if (e.down) LOAD = "down"; else LMSG = e.message;
  }
  LBUSY = false;
  renderLive();
}

/* ---- 文章 ---- */
function msgById(id) { return ST.messages.find((m) => m.id === id); }
function describe(a) {
  const m = msgById(a.message_id);
  if (a.type === "send") return nm(a.who) + " が『" + a.text + "』を送信して" + (a.delivered_now ? "届けた" : "待機中にした");
  if (a.type === "deliver") return nm(a.who) + " に #" + a.message_id + " を届けた";
  if (a.type === "tamper") return "Mallory が #" + a.message_id + " を書き換えて届けた";
  return "Mallory が #" + a.message_id + " を再送した";
}
function hasDelivery(a) { return a.type !== "send" || a.delivered_now; }
function mismatch(a) {
  if (!hasDelivery(a)) return false;
  if (a.expect === "any_error") return a.outcome === "ok";
  return a.expect !== a.outcome;
}
function captionOf(a) {
  if (!a) {
    return ["Alice が Bob の公開鍵の束から最初の鍵を作った",
      "Alice は Bob の公開鍵の束（署名つき・使い捨て・耐量子）を使って、最初のセッションを作りました。まだメッセージは送っていません。左で送ってみてください。"];
  }
  const m = msgById(a.message_id);
  const q = m ? "『" + m.text + "』" : "";
  const to = a.type === "send" ? nm(OTHER[a.who]) : nm(a.who);
  if (a.type === "tamper") {
    return a.outcome === "bad_mac"
      ? ["Mallory が #" + a.message_id + " を書き換えて届けたが、MAC不一致で弾かれた", "MAC は暗号文から計算した改ざん検知の印です。1バイト変わると合わなくなり、" + to + " は開く前に捨てます。元のメッセージは待機中のままです。"]
      : ["Mallory が #" + a.message_id + " を書き換えて届けた（結果：" + OUT_JA[a.outcome] + "）", "libsignal の実際の結果です。期待と違うときは、下の「テストを作る」で確かめられます。"];
  }
  if (a.type === "replay") {
    return a.outcome === "duplicate"
      ? ["Mallory が #" + a.message_id + " を再送したが、重複として弾かれた", "使った鍵はすぐ捨てるので、同じメッセージは二度と開けません。"]
      : ["Mallory が #" + a.message_id + " を再送した（結果：" + OUT_JA[a.outcome] + "）", "libsignal の実際の結果です。" + (a.error ? "エラー：" + a.error : "")];
  }
  if (a.type === "send" && !a.delivered_now) {
    return [nm(a.who) + " が " + q + " を暗号化した（通信路で待機中）", "libsignal の message_encrypt で暗号化し、送信の鎖を1つ進めました。まだ届いていません。「届ける」を押すと相手が読みます。"];
  }
  if (a.outcome === "ok") {
    return [(a.type === "send" ? nm(a.who) + " → " + to + " " : to + " が ") + q + (a.type === "send" ? " を暗号化して届けた" : " を読めた"),
      "受信側の libsignal が同じ鍵を作り、MAC を確かめてから開きました。右の「このステップで起きたこと」が実際の出来事です。"];
  }
  return [to + " は " + q + " を読めなかった（" + OUT_JA[a.outcome] + "）", "libsignal がエラーを返しました。" + (a.error ? "エラー：" + a.error : "") + " 失敗した段階と保存の結果は「受信の診断」に表示しています。"];
}

/* ---- 左：チャット ---- */
function arrivalRanks() {
  const sendAt = {}, delAt = {}, pos = {}, cnt = { alice: 0, bob: 0 };
  ST.script.forEach((a) => {
    if (a.type === "send") { sendAt[a.message_id] = a.i; if (a.delivered_now && a.outcome === "ok") delAt[a.message_id] = a.i; }
    else if (a.type === "deliver" && a.outcome === "ok" && delAt[a.message_id] === undefined) delAt[a.message_id] = a.i;
  });
  ST.messages.slice().sort((x, y) => x.id - y.id).forEach((m) => { cnt[m.from]++; pos[m.id] = cnt[m.from]; });
  const done = { alice: 0, bob: 0 }, sync = { alice: 0, bob: 0 }, out = {};
  ST.messages.filter((m) => m.status === "delivered").sort((x, y) => (x.delivered_order || 0) - (y.delivered_order || 0)).forEach((m) => {
    const s = m.from;
    done[s]++;
    out[m.id] = { rank: done[s] - sync[s], rel: pos[m.id] - sync[s] };
    const at = delAt[m.id] === undefined ? Infinity : delAt[m.id];
    const sent = ST.messages.filter((x) => x.from === s && (sendAt[x.id] === undefined ? Infinity : sendAt[x.id]) <= at).length;
    if (done[s] === sent) sync[s] = done[s];
  });
  return out;
}
function outChip(to, outcome, verb) {
  if (outcome === "ok") return '<span class="chip ok">&#10003; ' + nm(to) + " 読めた</span>";
  return '<span class="chip bad">' + nm(to) + " " + (verb || "弾いた") + "：" + esc(OUT_JA[outcome] || outcome) + "</span>";
}
function renderLiveChat() {
  const log = $("chat-log");
  const last = ST.last;
  const ranks = arrivalRanks();
  let h = '<div class="sys' + (!last ? " cur" : "") + '" data-cur="' + (!last ? 1 : 0) + '">Alice が Bob の公開鍵の束から最初の鍵を作った</div>';
  ST.script.forEach((a) => {
    const m = msgById(a.message_id);
    if (a.type === "deliver") return;
    const isLast = !!last && last.i === a.i;
    if (a.type === "send") {
      const cur = !!last && last.message_id === a.message_id && (last.type === "send" || last.type === "deliver");
      let chips;
      if (m.status === "delivered") {
        chips = '<span class="chip ok">&#10003; ' + nm(m.to) + " 読めた</span>";
        const r = ranks[m.id];
        if (r && r.rank !== r.rel) chips += '<span class="chip order">' + r.rank + "番目に届いた</span>";
      } else {
        chips = '<span class="chip pend">' + nm(m.to) + " 通信路で待機中</span>";
      }
      ST.script.filter((x) => x.message_id === m.id && x.outcome !== "ok" && (x.type === "deliver" || (x.type === "send" && x.delivered_now)))
        .forEach((x) => { chips += outChip(m.to, x.outcome); });
      h += '<div class="chat-item' + (cur ? " cur" : "") + '" data-cur="' + (cur ? 1 : 0) + '">' + portraitHtml(m.from, "p32") +
        '<div class="chat-main"><div class="chat-name"><span class="dot">' + nm(m.from) + '</span><span class="meta mono">#' + m.id + "</span></div>" +
        '<div class="bubble">' + esc(m.text) + "</div>" +
        '<div class="meta mono">counter ' + m.counter + " · " + esc(m.kind) + "</div>" +
        '<div class="meta mono">暗号文 ' + esc(m.ciphertext_hex.slice(0, 6)) + "…（" + m.ciphertext_len + "バイト）</div>" +
        '<div class="chips">' + chips + "</div>" +
        '<div class="acts">' +
        (m.status === "waiting"
          ? '<button type="button" class="abtn" id="deliver-' + m.id + '" data-act="deliver" data-id="' + m.id + '">届ける</button>' +
            '<button type="button" class="abtn mal" id="flip-' + m.id + '" data-act="tamper" data-id="' + m.id + '">Mallory：1バイト書き換えて届ける</button>'
          : '<button type="button" class="abtn mal" id="replay-' + m.id + '" data-act="replay" data-id="' + m.id + '">Mallory：もう一度送る</button>') +
        "</div></div></div>";
    } else {
      const text = a.type === "tamper" ? "#" + a.message_id + " の暗号文を1バイト書き換えた" : "#" + a.message_id + " をもう一度送った";
      h += '<div class="chat-item mal' + (isLast ? " cur" : "") + '" data-cur="' + (isLast ? 1 : 0) + '">' + portraitHtml("mallory", "p32") +
        '<div class="chat-main"><div class="chat-name"><span class="dot">Mallory</span><span class="tag bad">割り込み</span></div>' +
        '<div class="bubble">' + esc(text) + "</div>" +
        '<div class="chips">' + outChip(m ? m.to : OTHER[a.who], a.outcome) + "</div></div></div>";
    }
  });
  log.innerHTML = h;
  scrollToCur(log);
}

/* ---- 右：裏側 ---- */
function boxView(cur, prev) {
  const pm = {};
  (prev || []).forEach((b) => { pm[b.id] = b; });
  const out = cur.map((b) => {
    const o = { id: b.id, label: b.id.indexOf("recv:") === 0 ? b.label + "（相手のDH鍵 " + b.id.slice(5) + "）" : b.label, fp: b.fp, index: b.index };
    if (prev) {
      const p = pm[b.id];
      if (!p) { o.changed = true; o.tag = "新規"; }
      else if (p.fp !== b.fp || p.index !== b.index) {
        o.changed = true;
        if (b.id === "root") o.replacedFrom = p.fp;
      }
    }
    return o;
  });
  if (prev) {
    const ids = {};
    cur.forEach((b) => { ids[b.id] = true; });
    prev.forEach((p, pi) => {
      if (!ids[p.id]) {
        out.splice(Math.min(pi, out.length), 0, { id: p.id, label: p.label, fp: p.fp, index: p.index, old: true, tag: p.id.indexOf("skipped") === 0 ? "使用済み" : "置き換え前" });
      }
    });
  }
  return out;
}
function renderLiveStage() {
  const last = ST.last;
  let to = null;
  if (last) to = last.type === "send" ? (last.delivered_now ? OTHER[last.who] : null) : last.who;
  const bad = !!last && last.outcome !== "ok";
  const notes = {};
  if (to && bad) notes[to] = "弾かれた（" + (OUT_JA[last.outcome] || last.outcome) + "）。失敗した段階は受信の診断で確認できます";
  renderStage(SCENES.live, {
    state: {
      alice: boxView(ST.alice, PREV && PREV.alice), bob: boxView(ST.bob, PREV && PREV.bob), mallory: []
    },
    result: bad ? "bad" : "ok",
    message: to ? { to: [to], inflight: false } : null,
    notes, alert: to && bad ? [to] : []
  });
}

/* ---- 下：テストを作る ---- */
function renderTestPanel() {
  const script = ST.script;
  const empty = script.length === 0;
  $("test-empty").hidden = !empty;
  $("script-wrap").hidden = empty;
  let rows = "";
  script.forEach((a) => {
    const sel = hasDelivery(a);
    const mm = mismatch(a);
    let res = "—", exp = "—";
    if (sel) {
      res = '<span class="chip ' + (a.outcome === "ok" ? "ok" : "bad") + '" title="' + esc(a.error || "") + '">' + OUT_JA[a.outcome] + "</span>";
      exp = '<select id="expect-' + a.i + '" data-i="' + a.i + '" aria-label="こうなるはず（' + (a.i + 1) + '番）"' + (LBUSY ? " disabled" : "") + ">" +
        [["ok", "読める（ok）"], ["duplicate", "重複で弾かれる（duplicate）"], ["bad_mac", "MAC不一致で弾かれる（bad_mac）"], ["any_error", "何かのエラー（any_error）"]]
          .map((o) => '<option value="' + o[0] + '"' + (a.expect === o[0] ? " selected" : "") + ">" + o[1] + "</option>").join("") + "</select>" +
        (mm ? '<div class="warn">このままだとテストは失敗します</div>' : "");
    }
    rows += '<tr class="' + (mm ? "mismatch" : "") + '"><td class="mono">' + (a.i + 1) + "</td><td>" + esc(describe(a)) + "</td><td>" + res + "</td><td>" + exp + "</td></tr>";
  });
  $("script-body").innerHTML = rows;
  const name = $("test-name").value;
  const valid = NAME_RE.test(name);
  $("name-error").hidden = valid;
  $("btn-gen").disabled = empty || !valid || RUNNING || LBUSY;
  const canRun = !!GEN && !GEN.stale && GEN.name === name && valid && !RUNNING && !empty;
  $("btn-run").disabled = !canRun;
  $("run-hint").hidden = !(GEN && (GEN.stale || GEN.name !== name)) || empty;
  $("gen-result").hidden = !GEN;
  if (GEN) {
    $("gen-path").textContent = GEN.path;
    $("gen-code").textContent = GEN.code;
  }
  $("test-list").innerHTML = TESTS.length
    ? TESTS.map((t) => "<li><span class=\"mono\">" + esc(t.name) + '</span> <span class="meta mono">' + esc(t.path) + "</span></li>").join("")
    : '<li class="meta">まだありません</li>';
}

function renderLive() {
  const down = LOAD === "down";
  $("live-error").hidden = !down;
  $("panel-main").hidden = down;
  $("caption").hidden = down;
  $("json-details").hidden = down;
  $("test-panel").hidden = down || LOAD !== "ready" || !ST;
  $("receive-diagnosis").hidden = down || LOAD !== "ready" || !ST;
  $("chat-title").textContent = "Alice と Bob";
  $("btn-reset").disabled = LBUSY || LOAD !== "ready";
  if (down) return;
  const ready = LOAD === "ready" && !!ST;
  $("live-msg").textContent = LMSG;
  $("live-msg").hidden = !LMSG;
  $("composer").classList.toggle("busy", LBUSY || !ready);
  ["sender-alice", "sender-bob", "deliver-now", "live-text"].forEach((id) => { $(id).disabled = LBUSY || !ready; });
  ["alice", "bob"].forEach((id) => {
    $("sender-" + id).setAttribute("aria-checked", SENDER === id ? "true" : "false");
    $("sender-" + id).classList.toggle("on", SENDER === id);
  });
  $("btn-send").disabled = !ready || LBUSY || $("live-text").value.trim() === "";
  if (!ready) {
    $("chat-log").innerHTML = '<div class="sys">サーバーに問い合わせています…</div>';
    $("stage").innerHTML = "";
    $("happen-list").innerHTML = "";
    $("cap-title").textContent = "";
    $("cap-text").textContent = "";
    $("json-pre").textContent = "";
    return;
  }
  renderLiveChat();
  renderLiveStage();
  const diagnosis = diagnosisHtml(ST.last, ST.last ? msgById(ST.last.message_id) : null);
  $("receive-diagnosis").innerHTML = diagnosis;
  $("receive-diagnosis").hidden = !diagnosis;
  const cap = captionOf(ST.last);
  $("cap-title").textContent = cap[0];
  $("cap-text").textContent = cap[1];
  const evs = ST.last ? ST.last.events.filter((event) => event.kind !== "decrypt_stage") : [];
  $("happen-list").innerHTML = evs.length
    ? evs.map((e) => "<li>" + esc(evText(e, "live")) + '<span class="ev-tag">' + esc(e.kind) + "</span></li>").join("")
    : '<li class="meta">まだ出来事はありません</li>';
  $("json-pre").textContent = ST.last ? JSON.stringify(ST.last, null, 2) : "（まだ操作がありません）";
  renderTestPanel();
}

function initLive() {
  $("sender-alice").addEventListener("click", () => { SENDER = "alice"; renderLive(); });
  $("sender-bob").addEventListener("click", () => { SENDER = "bob"; renderLive(); });
  $("deliver-now").addEventListener("change", (e) => { DELIVER_NOW = e.target.checked; });
  $("live-text").addEventListener("input", () => { $("btn-send").disabled = LOAD !== "ready" || LBUSY || $("live-text").value.trim() === ""; });
  const send = () => {
    const text = $("live-text").value.trim();
    if (!text || LBUSY || LOAD !== "ready") return;
    runLive(async () => {
      setState(await api("POST", "/api/send", { from: SENDER, text, deliver: DELIVER_NOW }));
      $("live-text").value = "";
    });
  };
  $("btn-send").addEventListener("click", send);
  $("live-text").addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.isComposing) { e.preventDefault(); send(); }
  });
  $("btn-reset").addEventListener("click", () => runLive(async () => {
    const st = await api("POST", "/api/reset");
    ST = st; PREV = null; GEN = null;
  }));
  $("btn-retry").addEventListener("click", () => enterLive());
  $("chat-log").addEventListener("click", (e) => {
    const b = e.target.closest("button[data-act]");
    if (!b || sceneId !== "live") return;
    const act = b.dataset.act, id = Number(b.dataset.id);
    runLive(async () => { setState(await api("POST", "/api/" + act, { message_id: id })); });
  });
  $("script-body").addEventListener("change", (e) => {
    const s = e.target.closest("select[data-i]");
    if (!s) return;
    runLive(async () => { setState(await api("PUT", "/api/script/" + s.dataset.i + "/expect", { expect: s.value }), true); });
  });
  $("test-name").addEventListener("input", () => { if (ST) renderTestPanel(); });
  $("btn-gen").addEventListener("click", () => runLive(async () => {
    const name = $("test-name").value;
    const r = await api("POST", "/api/test/generate", { name });
    GEN = { name: r.name, path: r.path, code: r.code, stale: false };
    $("run-result").hidden = true;
    await loadTests();
  }));
  $("btn-copy").addEventListener("click", async () => {
    const btn = $("btn-copy");
    try { await navigator.clipboard.writeText(GEN.code); btn.textContent = "コピーしました"; }
    catch (e) {
      const r = document.createRange();
      r.selectNodeContents($("gen-code"));
      const sel = getSelection(); sel.removeAllRanges(); sel.addRange(r);
      btn.textContent = "選択しました（⌘C でコピー）";
    }
    setTimeout(() => { btn.textContent = "コピー"; }, 2000);
  });
  $("btn-run").addEventListener("click", async () => {
    if (RUNNING || !GEN) return;
    const name = GEN.name;
    RUNNING = true;
    const t0 = Date.now();
    $("run-result").hidden = true;
    $("run-status").hidden = false;
    const tick = () => { $("run-status").textContent = "実行中…（初回は数分かかります） " + Math.floor((Date.now() - t0) / 1000) + "秒"; };
    tick();
    runTimer = setInterval(tick, 1000);
    renderTestPanel();
    try {
      const r = await api("POST", "/api/test/run", { name });
      $("run-big").textContent = r.passed ? "✓ 成功" : "✗ 失敗";
      $("run-big").className = "run-big " + (r.passed ? "ok" : "bad");
      $("run-info").textContent = "所要 " + (r.duration_ms / 1000).toFixed(1) + " 秒 · exit " + r.exit_code;
      $("run-tail").textContent = r.output_tail || "";
      $("run-details").open = !r.passed;
      $("run-result").hidden = false;
    } catch (e) {
      $("run-big").textContent = "✗ 実行できませんでした";
      $("run-big").className = "run-big bad";
      $("run-info").textContent = e.down ? "サーバーにつながりません" : e.message;
      $("run-tail").textContent = "";
      $("run-details").open = false;
      $("run-result").hidden = false;
    }
    clearInterval(runTimer);
    RUNNING = false;
    $("run-status").hidden = true;
    renderTestPanel();
  });
}

(function init() {
  initLive();
  const tabs = $("tabs");
  SCENE_ORDER.forEach((id) => {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "tab";
    b.id = "tab-" + id;
    b.setAttribute("role", "tab");
    b.textContent = SCENES[id].tab;
    b.addEventListener("click", () => setScene(id));
    tabs.appendChild(b);
  });
  $("btn-prev").addEventListener("click", () => go(stepIdx - 1));
  $("btn-next").addEventListener("click", () => go(stepIdx + 1));
  document.addEventListener("keydown", (e) => {
    const t = e.target;
    if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable)) return;
    if (e.altKey || e.ctrlKey || e.metaKey || sceneId === "live") return;
    if (e.key === "ArrowLeft") { go(stepIdx - 1); e.preventDefault(); }
    else if (e.key === "ArrowRight") { go(stepIdx + 1); e.preventDefault(); }
  });
  render();
})();
