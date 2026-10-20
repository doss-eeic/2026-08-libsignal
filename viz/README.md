# viz：libsignal の鍵の見える化

libsignal の中で鍵がどう変わるかを、ブラウザで1ステップずつ見るための道具です。

- ①〜③：手作りのサンプルデータを再生する説明用の画面
- ④：手元で動かした本物の libsignal（`rust/protocol`）に、画面からメッセージを送る
- ⑤：複数人の Sender Keys 通信で、配布・受信・改ざん・再送を操作する
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
- 記録を入れている場所：`src/double_ratchet.rs`、`src/triple_ratchet.rs`、`src/ratchet/keys.rs`、`src/session.rs`、`src/session_management.rs`、`src/group_cipher.rs`（出来事は `viz_record!`、段階の結果は `viz_observe!` で検索）
- 記録するのは古典的な Double Ratchet の鍵だけです。v0.105 の送信は `triple_ratchet.rs` を通るので送信側の記録はそこにありますが、耐量子（SPQR）の鍵は記録しません
- 記録は「今のスレッド」に貯めます（`viz_trace::start` → libsignal の呼び出し → `viz_trace::take`）。この間に `.await` を挟むと記録が混ざるので、サーバーは各呼び出しを `now_or_never` で同期的に実行しています
- 「Mallory：1バイト書き換えて届ける」は、届ける途中のコピーだけを書き換えます。元のメッセージは待機中のまま残り、あとで普通に届けられます。MAC が合わない受信では libsignal は鍵の状態を変えません
- ④と⑤の「受信の診断」は、形式の読み取り・鍵の準備・認証・本文の復号・保存の結果を、実際の処理の記録から表示します。④の初回の `PreKey`、通常の `Whisper` と、⑤の `SenderKey` に対応しています
- 診断は受信全体の結果と段階の一覧をコンパクトに表示します。段階ごとの詳しい説明、内部エラーや記録した関数は、詳細を開いて確認できます
- 現在の候補が失敗して過去の候補が成功した場合は、それぞれの候補と受信全体の結果を分けます。候補内の鍵更新イベントは作業用の状態で行った試行も含み、保存の成功を意味しません
- 「未実行」は、その経路で失敗した段階より後にある処理です。記録が不足している場合は「記録なし」と表示し、成功や未実行を推測しません
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
| `decrypt_stage` | 復号の各段階を開始・完了した | なし（`stage`、`status`、`source`、必要なら`detail`を持つ） |
| `sender_key_created` / `sender_key_received` | 自分の Sender Key を作った／配布された Sender Key を保存した | 鎖の鍵の種（`index` は現在の番号） |
| `sender_key_distributed` | サーバーが Sender Key を参加者へ配布した（`to` が配布先） | 配布した鎖の鍵の種 |
| `sender_chain_step` | Sender Key の鎖が `from` → `to_index` に進んだ | 進んだ後の鎖の鍵 |
| `sender_message_key` | このメッセージの鍵を使った（送る側は暗号化、受け取る側は復号に。取り置きを作るだけのときは出ない） | メッセージ鍵の種 |
| `signature_checked` | グループメッセージの署名を確認した（`ok` が結果） | なし |
| `no_sender_key` | 送信者の Sender Key がないため弾いた | なし |

受信診断の `status` は `started` / `ok` / `error` です。
④の候補内の記録には `candidate`（その操作内で0から数える試行番号）と `candidate_kind`（`current` / `previous`）が付きます。
画面には途中の内部エラーと、最後に呼び出し元へ返ったエラーを分けて表示します。
認証の段階はMACだけでなくアドレス確認も含むため、「認証の検証（MAC・アドレス）」という名前です。
相手の信頼確認と保存は、通常メッセージとPreKeyで元の処理順が異なり、画面もそれぞれの経路に合わせています。

## ④ 改ざん・受信条件の実験

最初はAliceから送ります。最初のメッセージがBobへ届くと、Bobも選べるようになります。「最初から」に戻すと、送信者もAliceへ戻ります。

通常の配達は、これまでどおり「すぐ届ける」または「届ける」を使います。エラーを観察するときは「すぐ届ける」をオフにして送り、待機中メッセージの小さな選択欄で種類を選んで「実験」を押します。

| 実験 | 変えるもの | 実際に拒否する処理・段階 |
| --- | --- | --- |
| 本文の1ビット改ざん | 配達用コピーの暗号化本文の1ビット | `TripleRatchet::decrypt` の認証の検証 |
| 短すぎるメッセージ | 配達用コピー全体を0バイトにする（本文の文字数を減らす操作ではない） | `SignalMessage::try_from` / `PreKeySignalMessage::try_from` の形式の読み取り。`message_decrypt` には入らない |
| セッションなし | 受信側の保管庫のコピーでセッション保管庫だけを空にする | `message_decrypt_signal` の `load_session` 後の存在確認 |
| 相手の身元の鍵を信頼できない | 受信側の保管庫のコピーが覚えている相手の身元の鍵を別の鍵に置き換える。暗号文・セッションは維持 | `message_decrypt_signal` の `is_trusted_identity`。通常経路では認証・本文復号が成功してから拒否される |

後ろ2つは通常の `SignalMessage` 用です。Aliceの最初のメッセージを届ける → Bobから返信を届ける → Aliceから「すぐ届ける」をオフにして送る、の順で準備すると試せます。初回の `PreKeySignalMessage` では選択できず、APIから指定しても説明付きで拒否します。

受信側の条件を変える実験では、`InMemSignalProtocolStore::clone` で作ったコピーを使います。元の保管庫・元のメッセージ・配達済み件数は変えず、実験後に「届ける」を押して本来のメッセージを正常に受信できます。画面の鍵の保管庫は元の状態、受信の診断は実験用コピーで実行した記録であることを明記しています。

実験の準備は `viz/server/src/sim.rs` の `experiment` / `decrypt_index` / `decrypt` にあります。停止箇所は、既存の `sim::parse` と libsignal の `message_decrypt_signal` 内にある段階記録から得ます。エラーを表示用に作ったり、暗号処理や信頼判定を変更したりはしていません。`classify` は返ってきたエラー型を `too_short` / `no_session` / `untrusted_identity` へ分類するだけです。

`POST /api/tamper` の `mode` は `body_bit_flip`（省略時の従来動作）/ `truncate` / `no_session` / `untrusted_identity`。実験の種類は操作履歴の `tamper_mode` に保存され、「テストを作る」で生成するRustテストにも再現されます。

## ⑤ グループ（Sender Keys）

Alice と Bob から始め、Carol と Dave が後から参加できます。`/api/group/` で、本物の libsignal の Sender Keys による配布・送信・配達・改ざん・再送を操作します。参加者ごとに、各送信者の鎖の現在位置と、順序が逆に届いたときの取り置きの鍵を指紋で確認できます。`skipped_key_stored`、`skipped_key_used`、`duplicate_rejected` はグループでも同じ意味です。

Sender Key の配布メッセージはストア間で直接渡します。実際の Signal が使う1対1の暗号化メッセージによる配送は省略しています。参加だけでは鍵を配布せず、次の送信時に未配布の参加者へ現在の鍵を渡します。配布前の古いメッセージは `no_sender_key`、現在の鍵を受け取った後も古い番号は `duplicate` となり読めません。署名に失敗したコピーは `bad_signature` となり、元のメッセージは後で正常に配達できます。

グループの「テストを作る」も公開 API だけの Rust テストを生成し、既存の `/api/test/run` で実行します。

### ④と⑤の復号処理の違い

④の1対1通信は `message_decrypt` → `message_decrypt_signal` / `message_decrypt_prekey` → `TripleRatchet::decrypt` を通り、MACとアドレスを検証します。⑤のグループ本文は別の入口である `group_decrypt`（`rust/protocol/src/group_cipher.rs`）を通り、送信者が配布した Sender Key の記録を読み出し、電子署名を検証してからメッセージ用の鍵を準備します。⑤も同じ `message_decrypt` を使うわけではありません。

⑤で観察する順序は、`parse`（形式の読み取り）→ `sender_key_load`（Sender Keyの記録の取得）→ `sender_key_state`（該当する鎖の選択）→ `signature`（署名の検証）→ `key_prepare`（メッセージ用の鍵の準備）→ `plaintext`（本文の復号）→ `sender_key_save`（更新後の記録の保存）です。`receive` は呼び出し全体の結果です。これらは既存の処理を `viz_observe!` で囲んで記録しており、暗号方式、検証の判断や返すエラーは変更していません。

各受信者を指定して「届ける」「改ざんして届ける」「再送」を行うと、その受信者の直近の結果を診断に表示します。正常受信は保存まで成功、本文改ざんは署名で失敗し鍵の準備以降は未実行、受信済みメッセージの再送は署名に成功して鍵の準備で失敗します。鍵の未配布の参加者への配達は、記録の取得で失敗し、署名の検証にも進みません。これらを同じ表示で比較できます。

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
node viz/web/diagnosis.test.cjs                         # 受信診断の表示判定
```

この `viz/` と `viz-trace` は班のフォーク専用です。本家（signalapp/libsignal）への PR には含めません。
