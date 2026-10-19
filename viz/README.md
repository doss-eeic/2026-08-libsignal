# viz：libsignal の鍵の見える化

libsignal の中で鍵がどう変わるかを、ブラウザで1ステップずつ見るための道具です。

- ①〜③：手作りのサンプルデータを再生する説明用の画面
- ④：手元で動かした本物の libsignal（`rust/protocol`）に、画面からメッセージを送る
- ④の「テストを作る」：画面での操作から Rust のテストを生成し、その場で `cargo test` を実行する

## 起動（macOS / Linux）

前提：このリポジトリで `cargo test -p libsignal-protocol` が通る環境（Rust と protoc が入っていること）。

```bash
# リポジトリのルートで
cargo run --manifest-path viz/server/Cargo.toml
```

起動したらブラウザで http://127.0.0.1:8787 を開きます。初回はビルドに数分かかります。待ち受けは自分の PC だけ（127.0.0.1）です。`VIZ_ADDR=0.0.0.0:8787` のように変えられますが、そうすると同じネットワークの誰でもテストファイルを書き出して `cargo test` を実行できるので、教室の Wi-Fi などでは使わないでください（Docker の中で動かしたときだけ、自動で 0.0.0.0 になります）。

## 仕組み

```
ブラウザ（viz/web）
  │ /api/send, /api/deliver, /api/tamper, /api/replay …
  ▼
viz/server（Rust、axum）
  │ libsignal-protocol を viz-trace 機能付きで呼ぶ
  ▼
rust/protocol（本物の libsignal）
  └ viz_record! で記録した出来事と、鍵の状態（指紋）を返す
```

- `viz-trace` 機能を付けたときだけ記録が動きます。付けないビルドでは記録のコードは消え、libsignal の動作は変わりません
- 鍵そのものは画面に出しません。SHA-256 の先頭4バイト（8文字）を「指紋」として出します
- 記録を入れている場所：`src/double_ratchet.rs`、`src/triple_ratchet.rs`、`src/ratchet/keys.rs`、`src/session.rs`、`src/session_management.rs`（`viz_record!` で検索）
- 記録するのは古典的な Double Ratchet の鍵だけです。v0.105 の送信は `triple_ratchet.rs` を通るので送信側の記録はそこにありますが、耐量子（SPQR）の鍵は記録しません
- 記録は「今のスレッド」に貯めます（`viz_trace::start` → libsignal の呼び出し → `viz_trace::take`）。この間に `.await` を挟むと記録が混ざるので、サーバーは各呼び出しを `now_or_never` で同期的に実行しています
- 「Mallory：1バイト書き換えて届ける」は、届ける途中のコピーだけを書き換えます。元のメッセージは待機中のまま残り、あとで普通に届けられます。MAC が合わない受信では libsignal は鍵の状態を変えません
- API の詳細は `viz/server/src/main.rs` の各ハンドラを参照

| 出来事（kind） | いつ | `fp` が指す鍵 |
| --- | --- | --- |
| `session_created` | Alice が公開鍵の束から、Bob が最初のメッセージからセッションを作った | なし |
| `message_key` | このメッセージ専用の鍵を作った（`chain` = send / recv、`index` = 番号） | メッセージ鍵の種（seed） |
| `chain_step` | 鎖が `from` → `to` に進んだ | 進んだ後の鎖の鍵 |
| `root_step` | ルート鍵を新しくした | 新しいルート鍵（`old_fp` は前のもの） |
| `dh_ratchet` | 相手の新しい DH 公開鍵を受け取り、鎖を作り直した | 相手の新しい DH 公開鍵（`old_fp` は直前のもの） |
| `skipped_key_stored` / `skipped_key_used` | 先に届いたメッセージのために鍵を取り置いた／使った | なし（`index` が番号） |
| `duplicate_rejected` | 使い済みの番号なので弾いた | なし |
| `mac_rejected` | MAC が合わず弾いた | なし |
| `sender_key_created` / `sender_key_received` | 自分の Sender Key を作った／配布された Sender Key を保存した | 鎖の鍵の種（`index` は現在の番号） |
| `sender_key_distributed` | サーバーが Sender Key を参加者へ配布した（`to` が配布先） | 配布した鎖の鍵の種 |
| `sender_chain_step` | Sender Key の鎖が `from` → `to_index` に進んだ | 進んだ後の鎖の鍵 |
| `sender_message_key` | このメッセージの鍵を使った（送る側は暗号化、受け取る側は復号に。取り置きを作るだけのときは出ない） | メッセージ鍵の種 |
| `signature_checked` | グループメッセージの署名を確認した（`ok` が結果） | なし |
| `no_sender_key` | 送信者の Sender Key がないため弾いた | なし |

## ⑤ グループ（Sender Keys）

Alice と Bob から始め、Carol と Dave が後から参加できます。`/api/group/` で、本物の libsignal の Sender Keys による配布・送信・配達・改ざん・再送を操作します。参加者ごとに、各送信者の鎖の現在位置と、順序が逆に届いたときの取り置きの鍵を指紋で確認できます。`skipped_key_stored`、`skipped_key_used`、`duplicate_rejected` はグループでも同じ意味です。

Sender Key の配布メッセージはストア間で直接渡します。実際の Signal が使う1対1の暗号化メッセージによる配送は省略しています。参加だけでは鍵を配布せず、次の送信時に未配布の参加者へ現在の鍵を渡します。配布前の古いメッセージは `no_sender_key`、現在の鍵を受け取った後も古い番号は `duplicate` となり読めません。署名に失敗したコピーは `bad_signature` となり、元のメッセージは後で正常に配達できます。

グループの「テストを作る」も公開 API だけの Rust テストを生成し、既存の `/api/test/run` で実行します。

## テストを作る

1. ④で送信・配達・改ざん・再送を操作する
2. 「テストを作る」の表で、各操作に「こうなるはず」を選ぶ（最初は実際に起きた結果）
3. テスト名を入れて「テストに書き出す」→ `rust/protocol/tests/viz_<名前>.rs` ができる
4. 「実行」→ `cargo test -p libsignal-protocol --test viz_<名前>` の結果が出る

生成されたテストは `tests/session.rs` と同じ書き方（`mod support;` の補助関数を使う）で、libsignal の公開 API だけを使うので `viz-trace` なしで動きます。

注意：

- 「こうなるはず」の初期値は、今の libsignal が実際に返した結果です。そのまま書き出すと「今の動き」を固定するテストになり、libsignal にバグがあればそのバグを正解として焼き込みます。期待は仕様（Signal の文書やコードのコメント）を読んで自分で決めてください
- テスト名に既存の手書きテストと同じ名前（例：`trace` → `viz_trace.rs`）は使えません。上書きせずにエラーになります
- 「実行」は同時に1つだけです。実行中にもう一度押すと「テストを実行中です」になります。初回はビルドで数分かかります
- 生成したテストは、班のフォークの中でなら commit してかまいません。本家への PR に使うときは、`viz_` の名前やヘッダーのコメントを外し、既存の `tests/session.rs` などに合う形に書き直して、中身を自分で説明できる状態にしてから出してください

## 確かめるコマンド

```bash
cargo test -p libsignal-protocol                         # 記録なし（元のまま通ること）
cargo test -p libsignal-protocol --features viz-trace    # 記録あり（tests/viz_trace.rs を含む）
cargo test --manifest-path viz/server/Cargo.toml         # サーバー
```

この `viz/` と `viz-trace` は班のフォーク専用です。本家（signalapp/libsignal）への PR には含めません。
