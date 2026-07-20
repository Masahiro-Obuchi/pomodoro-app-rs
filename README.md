# pomodoro-app-rs

Rustの学習を兼ねて開発するポモドーロタイマーです。

最初にRatatuiを使ったLinux向けTUIを作り、その後egui/eframeを使ってLinuxネイティブ版とWebAssembly版を追加します。タイマーの状態遷移は、UIから独立した`pomodoro-core`に集約します。

詳しい仕様とマイルストーンは[`docs/IMPLEMENTATION_PLAN.md`](docs/IMPLEMENTATION_PLAN.md)を参照してください。

## TUIを実行する

```bash
cargo run -p pomodoro-tui
```

`Space`で開始・一時停止・再開、`r`でリセット、`n`で次のセッションへ進み、`q`で状態を保存して終了します。Linux通知には`notify-send`を利用し、利用できない場合もタイマーと履歴は動作します。

状態と履歴はXDG state directory配下の`pomodoro-app-rs/state.json`へ保存します。

## 開発用コマンド

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## ライセンス

[MIT License](LICENSE)
