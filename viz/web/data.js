/* ①〜③のサンプルデータ（手作り）。④は libsignal を動かすサーバーの出力を使う。 */
(function () {
/* ---- サンプルデータ（実運用では Rust が window.SCENES として書き出す想定） ---- */
const k = (id, label, index, fp, o) => Object.assign({ id, label, index, fp }, o || {});
const sk = (label, it, fp, o) => Object.assign({ id: "sk", label, index: it, fp, unit: "iteration" }, o || {});

window.SCENES = {
  "one-to-one": {
    title: "返信のたびに鍵の鎖が作り直される",
    tab: "① 1対1",
    cast: ["alice", "bob"],
    steps: [
      {
        actor: "alice", action: "最初の鍵交換（PQXDH）",
        caption: "最初の鍵交換で、2人だけが知るルート鍵ができる",
        explain: "4回のDH（鍵共有）と耐量子のKEMを混ぜて、AliceとBobだけが知る最初の鍵を作ります。2人のルート鍵は同じ色（同じ値）です。",
        message: null, result: "ok",
        events: [
          { kind: "root_step", who: "alice", source: "pqxdh", fp: "a4c91e27" },
          { kind: "root_step", who: "bob", source: "pqxdh", fp: "a4c91e27" },
          { kind: "chain_step", who: "alice", chain: "send", from: null, to: 0, fp: "3b7d0f52" }
        ],
        state: {
          alice: [k("root", "ルート鍵", 0, "a4c91e27", { changed: true, tag: "新規" }), k("send", "送信の鎖", 0, "3b7d0f52", { changed: true, tag: "新規" })],
          bob: [k("root", "ルート鍵", 0, "a4c91e27", { changed: true, tag: "新規" })]
        }
      },
      {
        actor: "alice", action: "Alice がメッセージを送る",
        caption: "Alice → Bob「こんにちは」 送信の鎖が 0 → 1 に進む",
        explain: "メッセージを1通送るたびに、鎖を1つ先へ進めて新しい鍵を作ります。使った鍵は残しません。",
        message: { from: "alice", to: ["bob"], text: "こんにちは", counter: "0", inflight: true }, result: "ok",
        events: [
          { kind: "message_key", who: "alice", index: 0, fp: "e3a9027c" },
          { kind: "chain_step", who: "alice", chain: "send", from: 0, to: 1, fp: "c812ae96" }
        ],
        state: {
          alice: [k("root", "ルート鍵", 0, "a4c91e27"), k("send", "送信の鎖", 1, "c812ae96", { changed: true })],
          bob: [k("root", "ルート鍵", 0, "a4c91e27")]
        }
      },
      {
        actor: "alice", action: "Alice がもう1通送る",
        caption: "Alice → Bob「元気？」 送信の鎖が 1 → 2 に進む",
        explain: "同じ要領でもう1回。1通ごとに別の鍵なので、1通目の鍵では2通目は開けません。",
        message: { from: "alice", to: ["bob"], text: "元気？", counter: "1", inflight: true }, result: "ok",
        events: [
          { kind: "message_key", who: "alice", index: 1, fp: "7b40d9e5" },
          { kind: "chain_step", who: "alice", chain: "send", from: 1, to: 2, fp: "5e0b73d4" }
        ],
        state: {
          alice: [k("root", "ルート鍵", 0, "a4c91e27"), k("send", "送信の鎖", 2, "5e0b73d4", { changed: true })],
          bob: [k("root", "ルート鍵", 0, "a4c91e27")]
        }
      },
      {
        actor: "bob", action: "Bob が2通を読む",
        caption: "Bob の受信の鎖が index 2 になり、Alice の送信の鎖と同じ色になる",
        explain: "BobはAliceと同じ計算を自分の側で1通ずつ進めます。鎖そのものは送らなくても、同じルート鍵から両側が同じ鎖を作れます。",
        message: { from: "alice", to: ["bob"], text: "こんにちは / 元気？", counter: "0, 1" }, result: "ok",
        events: [
          { kind: "chain_step", who: "bob", chain: "recv", from: 0, to: 1, fp: "c812ae96" },
          { kind: "message_key", who: "bob", index: 0, fp: "e3a9027c" },
          { kind: "chain_step", who: "bob", chain: "recv", from: 1, to: 2, fp: "5e0b73d4" },
          { kind: "message_key", who: "bob", index: 1, fp: "7b40d9e5" }
        ],
        state: {
          alice: [k("root", "ルート鍵", 0, "a4c91e27"), k("send", "送信の鎖", 2, "5e0b73d4")],
          bob: [k("root", "ルート鍵", 0, "a4c91e27"), k("recv", "受信の鎖", 2, "5e0b73d4", { changed: true, tag: "新規" })]
        }
      },
      {
        actor: "bob", action: "Bob が返信する（DHラチェット）",
        caption: "Bob → Alice「元気だよ」 返信でルート鍵が新しくなる",
        explain: "返信するとき、Bobは新しいDH鍵を作ってルート鍵を更新します。古い受信の鎖は残し、新しい送信の鎖を作ります。",
        message: { from: "bob", to: ["alice"], text: "元気だよ", counter: "0", inflight: true }, result: "ok",
        events: [
          { kind: "dh_ratchet", who: "bob", step: "send" },
          { kind: "root_step", who: "bob", from: "a4c91e27", to: "7f21d8b3" },
          { kind: "root_step", who: "bob", chain: "send", fp: "d90c46a1" },
          { kind: "message_key", who: "bob", index: 0, fp: "92e5b1f8" },
          { kind: "chain_step", who: "bob", chain: "send", from: 0, to: 1, fp: "d90c46a1" }
        ],
        state: {
          alice: [k("root", "ルート鍵", 0, "a4c91e27"), k("send", "送信の鎖", 2, "5e0b73d4")],
          bob: [
            k("root", "ルート鍵", 1, "7f21d8b3", { changed: true, replacedFrom: "a4c91e27" }),
            k("recv", "受信の鎖", 2, "5e0b73d4"),
            k("send", "送信の鎖", 1, "d90c46a1", { changed: true, tag: "新規" })
          ]
        }
      },
      {
        actor: "alice", action: "Alice が読む（DHラチェット）",
        caption: "Alice のルート鍵が Bob と同じ新しい値に変わる",
        explain: "返信のたびに鎖が作り直されるので、ある時点の鍵が漏れても、その先のメッセージは読めません。古い送信の鎖は役目を終えて捨てられます。",
        message: { from: "bob", to: ["alice"], text: "元気だよ", counter: "0" }, result: "ok",
        events: [
          { kind: "dh_ratchet", who: "alice", step: "recv" },
          { kind: "root_step", who: "alice", from: "a4c91e27", to: "7f21d8b3" },
          { kind: "chain_step", who: "alice", chain: "recv", from: 0, to: 1, fp: "d90c46a1" },
          { kind: "message_key", who: "alice", index: 0, fp: "92e5b1f8" }
        ],
        state: {
          alice: [
            k("root", "ルート鍵", 1, "7f21d8b3", { changed: true, replacedFrom: "a4c91e27" }),
            k("send_old", "送信の鎖", 2, "5e0b73d4", { old: true, tag: "破棄" }),
            k("recv", "受信の鎖", 1, "d90c46a1", { changed: true, tag: "新規" })
          ],
          bob: [
            k("root", "ルート鍵", 1, "7f21d8b3"),
            k("recv", "受信の鎖", 2, "5e0b73d4"),
            k("send", "送信の鎖", 1, "d90c46a1")
          ]
        }
      }
    ]
  },

  "group": {
    title: "1回の暗号化で全員が読める",
    tab: "② グループ",
    cast: ["alice", "bob", "carol", "dave"],
    steps: [
      {
        actor: "alice", action: "Alice が Sender Key を配る",
        caption: "Alice が Sender Key を作り、Bob と Carol に配る",
        explain: "Sender Key は「Alice がこのグループに送るときの鎖」です。配るときは、1対1の暗号で1人ずつ安全に届けます。",
        message: { from: "alice", to: ["bob", "carol"], text: "Sender Key の配布", counter: "iteration 0" }, result: "ok",
        notes: { dave: "まだ参加していない" }, dim: ["dave"],
        events: [
          { kind: "sender_chain_step", who: "alice", iteration: 0, fp: "e17a5c30" },
          { kind: "sender_key_distributed", from: "alice", to: ["bob", "carol"], iteration: 0, fp: "e17a5c30" },
          { kind: "sender_key_received", who: "bob", from: "alice", iteration: 0, fp: "e17a5c30" },
          { kind: "sender_key_received", who: "carol", from: "alice", iteration: 0, fp: "e17a5c30" }
        ],
        state: {
          alice: [sk("自分のSender Key", 0, "e17a5c30", { changed: true, tag: "新規" })],
          bob: [sk("AliceのSender Key", 0, "e17a5c30", { changed: true, tag: "受信" })],
          carol: [sk("AliceのSender Key", 0, "e17a5c30", { changed: true, tag: "受信" })],
          dave: []
        }
      },
      {
        actor: "alice", action: "Alice がグループに送る",
        caption: "Alice → グループ「集合は10時」 暗号化は1回だけ",
        explain: "Alice は1回だけ暗号化して送ります。Bob と Carol は同じ鎖を持っているので、それぞれ署名を確かめてから読めます。",
        message: { from: "alice", to: ["bob", "carol"], text: "集合は10時", counter: "iteration 0" }, result: "ok",
        notes: { dave: "まだ参加していない" }, dim: ["dave"],
        events: [
          { kind: "sender_chain_step", who: "alice", from: 0, to: 1, fp: "0b9d82f4" },
          { kind: "signature_checked", who: "bob", ok: true },
          { kind: "signature_checked", who: "carol", ok: true },
          { kind: "sender_chain_step", who: "bob", from: 0, to: 1, fp: "0b9d82f4" },
          { kind: "sender_chain_step", who: "carol", from: 0, to: 1, fp: "0b9d82f4" }
        ],
        state: {
          alice: [sk("自分のSender Key", 1, "0b9d82f4", { changed: true })],
          bob: [sk("AliceのSender Key", 1, "0b9d82f4", { changed: true })],
          carol: [sk("AliceのSender Key", 1, "0b9d82f4", { changed: true })],
          dave: []
        }
      },
      {
        actor: "alice", action: "Alice がもう1通送る",
        caption: "Alice → グループ「場所は駒場」 全員の鎖が iteration 2 へ",
        explain: "送るたびに鎖が1つ進み、受け取った側も同じだけ進めます。3人の鍵はずっと同じ色のままです。",
        message: { from: "alice", to: ["bob", "carol"], text: "場所は駒場", counter: "iteration 1" }, result: "ok",
        notes: { dave: "まだ参加していない" }, dim: ["dave"],
        events: [
          { kind: "sender_chain_step", who: "alice", from: 1, to: 2, fp: "74c3e1a8" },
          { kind: "signature_checked", who: "bob", ok: true },
          { kind: "signature_checked", who: "carol", ok: true },
          { kind: "sender_chain_step", who: "bob", from: 1, to: 2, fp: "74c3e1a8" },
          { kind: "sender_chain_step", who: "carol", from: 1, to: 2, fp: "74c3e1a8" }
        ],
        state: {
          alice: [sk("自分のSender Key", 2, "74c3e1a8", { changed: true })],
          bob: [sk("AliceのSender Key", 2, "74c3e1a8", { changed: true })],
          carol: [sk("AliceのSender Key", 2, "74c3e1a8", { changed: true })],
          dave: []
        }
      },
      {
        actor: "alice", action: "Dave が途中から参加する",
        caption: "Dave が参加。Alice は「今の」Sender Key を Dave に渡す",
        explain: "渡すのは現在の位置（iteration 2）の鍵だけです。それ以前の鍵は鎖を戻せないので、Dave には作れません。",
        message: { from: "alice", to: ["dave"], text: "Sender Key の配布", counter: "iteration 2" }, result: "ok",
        events: [
          { kind: "sender_key_distributed", from: "alice", to: ["dave"], iteration: 2, fp: "74c3e1a8" },
          { kind: "sender_key_received", who: "dave", from: "alice", iteration: 2, fp: "74c3e1a8" }
        ],
        state: {
          alice: [sk("自分のSender Key", 2, "74c3e1a8")],
          bob: [sk("AliceのSender Key", 2, "74c3e1a8")],
          carol: [sk("AliceのSender Key", 2, "74c3e1a8")],
          dave: [sk("AliceのSender Key", 2, "74c3e1a8", { changed: true, tag: "受信" })]
        }
      },
      {
        actor: "alice", action: "Alice がグループに送る",
        caption: "Alice → グループ「遅れないで」 4人とも iteration 3",
        explain: "Dave も同じ鎖に加わったので、ここからのメッセージは全員が読めます。",
        message: { from: "alice", to: ["bob", "carol", "dave"], text: "遅れないで", counter: "iteration 2" }, result: "ok",
        events: [
          { kind: "sender_chain_step", who: "alice", from: 2, to: 3, fp: "a62f09bd" },
          { kind: "signature_checked", who: "bob", ok: true },
          { kind: "signature_checked", who: "carol", ok: true },
          { kind: "signature_checked", who: "dave", ok: true }
        ],
        state: {
          alice: [sk("自分のSender Key", 3, "a62f09bd", { changed: true })],
          bob: [sk("AliceのSender Key", 3, "a62f09bd", { changed: true })],
          carol: [sk("AliceのSender Key", 3, "a62f09bd", { changed: true })],
          dave: [sk("AliceのSender Key", 3, "a62f09bd", { changed: true })]
        }
      },
      {
        actor: "dave", action: "Dave が過去のメッセージを読もうとする",
        caption: "Dave は参加前のメッセージ（iteration 0）を読めない",
        explain: "鎖は前にしか進められないので、途中参加者は過去のメッセージを読めません。これは弱点ではなく、狙い通りの動きです。",
        message: { from: "alice", to: ["dave"], text: "集合は10時", counter: "iteration 0", reason: "古い鍵は持っていない" }, result: "duplicate",
        notes: { dave: "iteration 0 の鍵は持っていない" }, alert: ["dave"],
        events: [
          { kind: "duplicate_rejected", who: "dave", iteration: 0, current: 3, reason: "old_iteration" }
        ],
        state: {
          alice: [sk("自分のSender Key", 3, "a62f09bd")],
          bob: [sk("AliceのSender Key", 3, "a62f09bd")],
          carol: [sk("AliceのSender Key", 3, "a62f09bd")],
          dave: [sk("AliceのSender Key", 3, "a62f09bd")]
        }
      }
    ]
  },

  "attacks": {
    title: "おかしなメッセージは弾かれる",
    tab: "③ 攻撃・異常",
    cast: ["alice", "mallory", "bob"],
    steps: [
      {
        actor: "alice", action: "Alice が3通送る",
        caption: "Alice が #1 #2 #3 を続けて送る 送信の鎖は 0 → 3",
        explain: "3通ぶんの鍵を順に作って送ります。ここまでは普通のやり取りです。",
        message: { from: "alice", to: ["bob"], text: "#1 #2 #3", counter: "0, 1, 2", inflight: true }, result: "ok",
        events: [
          { kind: "message_key", who: "alice", index: 0, fp: "6c1f3e88" },
          { kind: "chain_step", who: "alice", chain: "send", from: 0, to: 1, fp: "8d05fa63" },
          { kind: "message_key", who: "alice", index: 1, fp: "d4a2905b" },
          { kind: "chain_step", who: "alice", chain: "send", from: 1, to: 2, fp: "b3c7219e" },
          { kind: "message_key", who: "alice", index: 2, fp: "2f8e61a7" },
          { kind: "chain_step", who: "alice", chain: "send", from: 2, to: 3, fp: "4a6e90d1" }
        ],
        state: {
          alice: [k("root", "ルート鍵", 0, "c5d81a40"), k("send", "送信の鎖", 3, "4a6e90d1", { changed: true })],
          mallory: [],
          bob: [k("root", "ルート鍵", 0, "c5d81a40")]
        }
      },
      {
        actor: "bob", action: "#3 が先に届く",
        caption: "届く順番が入れ替わり、#3 が先に届く",
        explain: "Bob は #3 を開くために鎖を index 3 まで進め、飛ばした #1 #2 の鍵は「取り置き」として保存します。",
        message: { from: "alice", to: ["bob"], text: "#3", counter: "2" }, result: "ok",
        events: [
          { kind: "skipped_key_stored", who: "bob", index: 0, fp: "6c1f3e88" },
          { kind: "skipped_key_stored", who: "bob", index: 1, fp: "d4a2905b" },
          { kind: "chain_step", who: "bob", chain: "recv", from: 0, to: 3, fp: "4a6e90d1" },
          { kind: "message_key", who: "bob", index: 2, fp: "2f8e61a7" }
        ],
        state: {
          alice: [k("root", "ルート鍵", 0, "c5d81a40"), k("send", "送信の鎖", 3, "4a6e90d1")],
          mallory: [],
          bob: [
            k("root", "ルート鍵", 0, "c5d81a40"),
            k("recv", "受信の鎖", 3, "4a6e90d1", { changed: true, tag: "新規" }),
            k("skip0", "取り置きの鍵", 0, "6c1f3e88", { changed: true, tag: "保存" }),
            k("skip1", "取り置きの鍵", 1, "d4a2905b", { changed: true, tag: "保存" })
          ]
        }
      },
      {
        actor: "bob", action: "#1 が届く",
        caption: "#1 が届く 取り置きの鍵（index 0）で開けて、その鍵は捨てる",
        explain: "遅れて届いたメッセージは、保存しておいた鍵で開けます。使った鍵はその場で消えます。",
        message: { from: "alice", to: ["bob"], text: "#1", counter: "0" }, result: "ok",
        events: [
          { kind: "skipped_key_used", who: "bob", index: 0, fp: "6c1f3e88" }
        ],
        state: {
          alice: [k("root", "ルート鍵", 0, "c5d81a40"), k("send", "送信の鎖", 3, "4a6e90d1")],
          mallory: [],
          bob: [
            k("root", "ルート鍵", 0, "c5d81a40"),
            k("recv", "受信の鎖", 3, "4a6e90d1"),
            k("skip1", "取り置きの鍵", 1, "d4a2905b")
          ]
        }
      },
      {
        actor: "bob", action: "#2 が届く",
        caption: "#2 が届く 取り置きの鍵（index 1）で開けて、全部使い切る",
        explain: "これで取り置きの鍵はなくなりました。順番が入れ替わっても、全部のメッセージが読めます。",
        message: { from: "alice", to: ["bob"], text: "#2", counter: "1" }, result: "ok",
        events: [
          { kind: "skipped_key_used", who: "bob", index: 1, fp: "d4a2905b" }
        ],
        state: {
          alice: [k("root", "ルート鍵", 0, "c5d81a40"), k("send", "送信の鎖", 3, "4a6e90d1")],
          mallory: [],
          bob: [k("root", "ルート鍵", 0, "c5d81a40"), k("recv", "受信の鎖", 3, "4a6e90d1")]
        }
      },
      {
        actor: "mallory", action: "Mallory が #2 を再送する",
        caption: "Mallory が #2 をもう一度送る 重複として弾かれる",
        explain: "使った鍵はすぐ捨てるので、同じメッセージは二度と開けません。Bob の鍵の状態も変わりません。",
        message: { from: "mallory", to: ["bob"], text: "#2（再送）", counter: "1" }, result: "duplicate",
        alert: ["bob"], notes: { bob: "index 1 の鍵はもう無い" },
        events: [
          { kind: "duplicate_rejected", who: "bob", index: 1, reason: "key_already_used" }
        ],
        state: {
          alice: [k("root", "ルート鍵", 0, "c5d81a40"), k("send", "送信の鎖", 3, "4a6e90d1")],
          mallory: [],
          bob: [k("root", "ルート鍵", 0, "c5d81a40"), k("recv", "受信の鎖", 3, "4a6e90d1")]
        }
      },
      {
        actor: "mallory", action: "Mallory が #4 を書き換える",
        caption: "Mallory が #4 の暗号文を1バイト書き換える MAC不一致で弾かれる",
        explain: "メッセージには MAC という改ざん検知の印が付いています。1バイト変えるだけで印が合わなくなり、Bob は開く前に捨てます。",
        message: { from: "mallory", to: ["bob"], text: "#4（1バイト改ざん）", counter: "3" }, result: "bad_mac",
        alert: ["bob"], notes: { bob: "MAC が合わないので鎖は進めない" },
        events: [
          { kind: "chain_step", who: "alice", chain: "send", from: 3, to: 4, fp: "f20b8c57" },
          { kind: "mac_rejected", who: "bob", index: 3, expected: "5d1c88a2", got: "5d1c88a3" }
        ],
        state: {
          alice: [k("root", "ルート鍵", 0, "c5d81a40"), k("send", "送信の鎖", 4, "f20b8c57", { changed: true })],
          mallory: [],
          bob: [k("root", "ルート鍵", 0, "c5d81a40"), k("recv", "受信の鎖", 3, "4a6e90d1")]
        }
      },
      {
        actor: "mallory", action: "グループの署名を書き換える",
        caption: "グループのメッセージの署名を偽造すると、署名不一致で弾かれる",
        explain: "グループのメッセージには送信者の署名が付いています。Mallory は秘密の署名鍵を持たないので偽造できません。ここはチームのテスト PR が確かめる箇所と同じです。",
        message: { from: "mallory", to: ["bob"], text: "「集合は10時」（署名を書き換え）", counter: "iteration 3" }, result: "bad_signature",
        alert: ["bob"], notes: { bob: "署名の検証に失敗" },
        events: [
          { kind: "signature_checked", who: "bob", ok: false, reason: "signature_mismatch" }
        ],
        state: {
          alice: [k("root", "ルート鍵", 0, "c5d81a40"), k("send", "送信の鎖", 4, "f20b8c57")],
          mallory: [],
          bob: [
            k("root", "ルート鍵", 0, "c5d81a40"),
            k("recv", "受信の鎖", 3, "4a6e90d1"),
            sk("AliceのSender Key", 3, "a62f09bd", { changed: true, tag: "追加" })
          ]
        }
      }
    ]
  }
};

/* ---- チャット表示用データ（at = 現れるステップ番号、tl = [[ステップ番号, 状態, 追加情報]...]） ---- */
const C = (to, tl, o) => Object.assign({ to, tl }, o || {});
window.CHAT = {
  "one-to-one": {
    title: "Alice と Bob",
    items: [
      { at: 0, kind: "sys", text: "Alice と Bob が最初の鍵を交換した" },
      { at: 1, kind: "msg", who: "alice", text: "こんにちは", meta: "counter 0", chips: [C("bob", [[1, "pend"], [3, "ok"]])] },
      { at: 2, kind: "msg", who: "alice", text: "元気？", meta: "counter 1", chips: [C("bob", [[2, "pend"], [3, "ok"]])] },
      { at: 4, kind: "msg", who: "bob", text: "元気だよ", meta: "counter 0", chips: [C("alice", [[4, "pend"], [5, "ok"]])] }
    ]
  },
  "group": {
    title: "週末の集まり（4人）",
    items: [
      { at: 0, kind: "sys", text: "Alice が Sender Key を Bob と Carol に配った" },
      { at: 1, kind: "msg", who: "alice", text: "集合は10時", meta: "iteration 0", chips: [C("bob", [[1, "ok"]]), C("carol", [[1, "ok"]])] },
      { at: 2, kind: "msg", who: "alice", text: "場所は駒場", meta: "iteration 1", chips: [C("bob", [[2, "ok"]]), C("carol", [[2, "ok"]])] },
      { at: 3, kind: "sys", text: "Dave が参加した（今の Sender Key を受け取った）" },
      { at: 4, kind: "msg", who: "alice", text: "遅れないで", meta: "iteration 2", chips: [C("bob", [[4, "ok"]]), C("carol", [[4, "ok"]]), C("dave", [[4, "ok"]])] },
      { at: 5, kind: "sys", text: "Dave が参加前のメッセージ『集合は10時』を開こうとした", chips: [C("dave", [[5, "bad", "古い鍵は持っていない"]], { verb: "読めない" })] }
    ]
  },
  "attacks": {
    title: "Alice と Bob",
    items: [
      { at: 0, kind: "msg", who: "alice", text: "#1", meta: "counter 0", chips: [C("bob", [[0, "pend"], [2, "ok", "2番目に届いた"]])] },
      { at: 0, kind: "msg", who: "alice", text: "#2", meta: "counter 1", chips: [C("bob", [[0, "pend"], [3, "ok", "3番目に届いた"]])] },
      { at: 0, kind: "msg", who: "alice", text: "#3", meta: "counter 2", chips: [C("bob", [[0, "pend"], [1, "ok", "1番目に届いた"]])] },
      { at: 4, kind: "msg", who: "mallory", mal: true, text: "#2 をもう一度送った", chips: [C("bob", [[4, "bad", "重複"]], { verb: "弾いた" })] },
      { at: 5, kind: "msg", who: "mallory", mal: true, text: "#4 の暗号文を1バイト書き換えた", chips: [C("bob", [[5, "bad", "MAC不一致"]], { verb: "弾いた" })] },
      { at: 6, kind: "msg", who: "mallory", mal: true, text: "グループのメッセージの署名を偽造した", chips: [C("bob", [[6, "bad", "署名不一致"]], { verb: "弾いた" })] }
    ]
  }
};

})();
