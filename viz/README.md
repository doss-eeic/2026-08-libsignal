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

起動したらブラウザで http://127.0.0.1:8787 を開きます。初回はビルドに数分かかります。待ち受けは自分の PC だけ（127.0.0.1）です。変えるときは `VIZ_ADDR=0.0.0.0:8787` のように指定します。

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
- API の詳細は `viz/server/src/main.rs` の各ハンドラを参照

## テストを作る

1. ④で送信・配達・改ざん・再送を操作する
2. 「テストを作る」の表で、各操作に「こうなるはず」を選ぶ（最初は実際に起きた結果）
3. テスト名を入れて「テストに書き出す」→ `rust/protocol/tests/viz_<名前>.rs` ができる
4. 「実行」→ `cargo test -p libsignal-protocol --test viz_<名前>` の結果が出る

生成されたテストは `tests/session.rs` と同じ書き方（`mod support;` の補助関数を使う）です。PR に使うときは、中身を読んで自分で説明できる状態にしてから、名前やコメントを整えてください。

## 確かめるコマンド

```bash
cargo test -p libsignal-protocol                         # 記録なし（元のまま通ること）
cargo test -p libsignal-protocol --features viz-trace    # 記録あり（tests/viz_trace.rs を含む）
cargo test --manifest-path viz/server/Cargo.toml         # サーバー
```

この `viz/` と `viz-trace` は班のフォーク専用です。本家（signalapp/libsignal）への PR には含めません。
