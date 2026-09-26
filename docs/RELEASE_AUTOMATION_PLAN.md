# 自動リリース整備計画

作成日（日本時間）：2026-09-27

状態：計画策定。配布用workflow、タグ、GitHub Releaseの作成は未着手。

## 1. 出発点と最初の公開範囲

- [CI拡充計画](CI_EXPANSION_PLAN.md)のCI-1～5は完了し、mainへのマージにはRust 1.86の4 OSとLinux stableの計5件の成功が必要。配布用workflowはまだない。
- `pomodoro-tui`の版はworkspaceの`0.1.0`を使う。計画時点でGitHub Releaseとタグはない。READMEが対応済みと案内しているのはLinuxのみ。
- 最初はGitHub Releasesに**Linux x86_64のプレリリース**を自動公開する。初回タグ候補は`v0.1.0`。GitHub上のプレリリース指定とCargoの版を区別し、タグの`v`を除いた値が`pomodoro-tui`の版と完全一致するようにする。
- 公開のきっかけは、5件のmain push検証が成功したコミットへの**手動タグ付け**とする。mainの各マージから自動で版やタグを作らない。タグのpush後、配布workflowが検証、ビルド、公開まで行う。
- 初回の配布対象はUbuntu 24.04 x86_64 runnerでビルド・確認した`x86_64-unknown-linux-gnu`のみ。必要なローカルファイルシステムと任意の`notify-send`を利用案内に記し、他のLinux環境での動作をCI結果だけから保証しない。
- crates.io公開、インストーラー、自動更新、Windows ARM、署名・公証はこの初回リリースに含めない。Windows/macOSのバイナリを対応版として公開する条件は§5に置く。

## 2. 配布物と公開条件

初回リリースには次を載せる。名前にはタグと対象tripleを含め、同じ版の配布物を後から差し替えない。

| 配布物 | 内容・検証 |
| --- | --- |
| `pomodoro-tui-v0.1.0-x86_64-unknown-linux-gnu.tar.gz` | `cargo +1.86.0 build --release --locked -p pomodoro-tui`でタグのコミットから作った実行ファイル、`LICENSE`、`README.md`。展開した**配布用実行ファイル**を隔離した保存先のPTYで起動・操作・正常終了する |
| `SHA256SUMS` | アーカイブのSHA-256。公開後にダウンロードしたファイルと照合する。チェックサムを再現可能ビルドの証明とは扱わない |
| リリースノート | `docs/releases/v0.1.0.md`をタグより前のPRでレビューする。対象OS・端末、起動方法、保存先、通知条件、既知の制限を記し、公開時にタグのcommit SHAを追記する |

配布workflowは公開前に次の条件を**すべて**検査し、欠けた場合は失敗して公開しない。

1. 既存の`vX.Y.Z`タグを対象とし、タグのcommitが`main`の履歴に含まれる。`pomodoro-tui`のCargo版とタグが一致する。初回以降の版上げは別PRで行う。
2. **同じcommit SHA**に対する`main`へのpushの`platform-probe.yml`実行が成功し、必須の5 jobがそれぞれ成功している。PR上の成功や別commitの成功を流用しない。結果が保留中なら公開せず、成功後にworkflowを再実行する。
3. 対応するリリースノートが存在し、対象範囲と制限を明記している。対象タグのGitHub Releaseがドラフトを含めて未作成である。
4. 配布用実行ファイルのPTYスモークテスト、アーカイブの展開、ファイル名、チェックサム検査が成功する。ビルド時のRust/Cargo版とcommit SHAをログに残す。

公開jobだけに`contents: write`を与え、検証・ビルドjobは読込権限に限定する。配布物は同一workflow内のartifactで公開jobへ渡す。既存のPR/main CIの読込専用tokenと5件のcheck名は維持する。[GitHub Actionsの権限](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#permissions)、[workflow artifact](https://docs.github.com/en/actions/concepts/workflows-and-actions/workflow-artifacts)

## 3. 実行順序とレビュー単位

| 順序 | 実装・設定 | 完了の証拠 |
| --- | --- | --- |
| REL-1：配布物の組立と検査 | Linux用のパッケージ作成手順、版・ファイル名の検査、展開後の実行ファイルを使うPTYスモークテストを追加する。`platform-probe.yml`のRust 1.86 Linux jobで`--release --locked`のビルドとパッケージ検査を実行する。初回リリースノートとLinux配布手順をPRでレビューする | 既存の必須5 checkがPRとmain pushで成功し、Linux jobで配布物を検査できる。check名と増えた実行時間を記録する。GitHub Releaseは作らない |
| REL-2：タグ用workflow | `.github/workflows/release.yml`を追加する。`workflow_dispatch`は公開しない検証実行とし、`v*`タグのpushで§2の条件を検査する。検証・ビルドが成功したときだけ別の公開jobが`gh release create`でプレリリースと配布物を作る | main上の手動検証実行でアーカイブとチェックサムを取得・照合でき、GitHub Releaseが作られない。版違い・main外・必須job未成功・既存Releaseを、公開せずに拒否する検査がある。既存CIの5 jobは維持される |
| REL-3：タグとReleaseの保護 | `v*`を対象にしたタグrulesetで更新・削除を制限し、Releaseのimmutable設定を初回公開前に有効にする。既存main rulesetは変更しない | 有効なrulesetの対象パターンと作成・更新・削除の設定、Releaseのimmutable設定を記録する。公開済みReleaseの配布物を後から変更しない運用が確定する |
| REL-4：初回公開と実物確認 | REL-1～3のPR・設定・main push検証後、`main`上の確認済みcommitに`v0.1.0`を付ける。タグworkflowによるLinuxプレリリースの公開を待ち、ReleaseからダウンロードしてSHA-256と展開・起動を確認する | Release URL、タグ/commit SHA、5件のmain push CI run、配布workflow run、アーカイブ名とチェックサム、ダウンロード後の起動結果を記録する。失敗時は公開済みの資産を自動上書きしない |

`gh release create`には`--verify-tag`、`--prerelease`、`--notes-file`を使い、存在しないタグの暗黙作成を防ぐ。アップロード途中でドラフトが残った場合は、状態を確認してから人が復旧方法を決める。公開済みの同じタグに対する再実行は拒否し、修正版は新しい版とタグで発行する。[GitHub CLIのrelease作成](https://cli.github.com/manual/gh_release_create)、[immutable releases](https://docs.github.com/en/code-security/how-tos/secure-your-supply-chain/establish-provenance-and-integrity/prevent-release-changes)

## 4. 検証手順

- REL-1のPRでは、既存5 checkとLinux配布物の組立・展開後のPTY操作を確認する。配布用バイナリの起動・保存・終了を、開発用debugバイナリのテスト結果だけで代用しない。
- REL-2のPRではタグ・版・main所属・CI結果・既存Releaseの判定を公開処理から分けて検証する。タグを実際に作らず、不正条件では公開jobに到達しないことを確認する。
- REL-2をマージした後、`workflow_dispatch`をmainで実行してアーカイブとチェックサムをダウンロードし、Releaseが作られていないことを確認する。
- REL-4でタグをpushした後、公開されたReleaseにLinuxアーカイブと`SHA256SUMS`が1件ずつ存在し、タグのcommit SHA・リリースノート・実物のハッシュと起動結果が一致することを確認する。
- 失敗、保留、重複タグのいずれでも配布物を公開・差し替えない。失敗したworkflowは原因を修正し、必要なら新しい版でやり直す。

## 5. Windows/macOSの追加条件

[W5b実端末受入](WINDOWS_MACOS_W5_ACCEPTANCE.md)が未完了のため、初回はWindows/macOSの配布物を公開しない。各OSを追加する前に、対象OS・端末の版を固定し、**そのOS向けにパッケージ化した実行ファイルそのもの**で起動、主要キー・貼付け・リサイズ、通知の実表示、保存失敗・復旧、終了後の端末復元を確認する。結果と制限をW5b記録とREADMEに反映し、受入が通った組合せだけを次の版のReleaseへ追加する。既存の`v0.1.0`配布物には後付けしない。

## 6. この計画の完了条件

- Linux向けのタグ起動プレリリースが、mainの同じSHAの5件成功、版一致、実物の検査を満たす場合にだけ公開される。
- 公開job以外に書込権限を与えず、PRから公開できない。タグ・Releaseの更新と削除が保護される。
- 配布物をダウンロードした利用者が、チェックサムを照合し、記載した環境で起動できる。公開runと実物確認の証拠を残す。
- Windows/macOSを対応済みと案内する前にW5bを完了し、次の版で受入済みの配布物だけを追加する。
