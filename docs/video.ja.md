# 動画編集

エージェントと CLI は `columbina-engine` を通じて Studio プロジェクトを編集します。
プロジェクト、編集規則、取り消し履歴、再生、書き出しはエンジンが管理します。
Python は文字起こしと編集用の原稿を扱い、ffmpeg フィルターグラフによる別の動画生成は行いません。

## 準備

- Windows。エンジンは現在 Windows のみに対応しており、動画ツール、`columbina video`、デスクトップアプリの編集スタジオも Windows でだけ使えます。
  macOS には今後対応します。macOS と Linux ではツールを提供せず、`columbina video` の各コマンドはその旨を表示します。エージェントのほかの機能はそのまま使えます。
- `COLUMBINA_ENGINE` にインストール済みエンジンの絶対パスを指定するか、`columbina-engine` を PATH に置きます。
  作業ディレクトリ内の実行ファイルは使いません。動画プラグインの `engine` 設定でも指定できます。
  [プロトコル 2](../engine/PROTOCOL.md)を参照してください。ソースからのビルドは FFmpeg 開発ライブラリを設定し、
  `engine/` で `cargo build --release -p columbina-server` を実行します。
- エンジンには LGPL FFmpeg の共有ライブラリが必要です。システムの **ffmpeg と ffprobe** は音声抽出と無音解析に使います。
- faster-whisper は別途インストールし、ダウンロード済みのモデルを `COLUMBINA_WHISPER_MODEL` で指定します。
  モデルは自動ダウンロードしません。`transcribe --words FILE` なら `start_us`、`end_us`、`text` を含む JSON 配列を読み込みます。
- `COLUMBINA_VIDEO=1` で動画ツールを有効にします。音声認識はローカルで実行します。文字起こし用に取り出した音声は、
  単語を保存した後（または失敗した時）に削除します。`analysis/` には素材ごとの単語と無音情報だけが残ります。

## 共有プロジェクト

ツールは `video_import`、`video_info`、`video_edit`、`video_transcribe`、`video_story`、`video_apply`、
`video_correct`、`video_render`、`video_undo` の 9 個です。既定のプロジェクトは作業ディレクトリの
`.columbina/studio` で、`project` で別の場所を指定できます。`video_info` からメディア、タイムライン、トラック、クリップの ID を得ます。
インポートはメディアプールへの登録だけを行います。クリップは `video_edit` で配置し、バッチ全体を
**1 個の原子的トランザクション、1 回の取り消し単位**として適用します。デスクトップとエージェントは履歴を共有します。
取り消しは、もう一方のクライアントが直前に行った編集も対象にします。

挿入、上書き、置き換え、上のトラックへの配置、末尾への追加、リップル削除、ブレード、トリム、ロール、スリップ、スライド、
速度変更、トランジションなど、DaVinci の編集概念を使います。ロック、リンク、素材の余裕はエンジンが判断します。
タイムラインの位置は整数フレーム、素材内の位置は ticks と scale です。省略したフィールドは既定の規則に従い、
明示的な `null` は値の消去または配置先の無効化を意味します。

`video_story` は会話トラック（既定は A1）を `story/<タイムライン ID>.md` に書き出します。
単語や行を `~~` で消すか削除し、`video_apply` で未ロックのトラックをリップル削除します。
トリム、繰り返し配置、速度変更、逆再生後の位置にも対応します。行の移動による並べ替えや、発話にない単語の追加はできません。
`[pause 1.8s]` の変更で間を短縮でき、`dry=true` では削除範囲を確認するだけです。
文字や数字を含まない記号（`—`、`♪`、`...` など）は同じ行の最も近い単語に従います。前に単語があればその単語、
行頭なら次の単語です。記号だけの行は、行を削除するか空にするか `~~` で消した場合にだけカットされます。
変更していない原稿は何もカットしません。
タイムラインや文字起こしが変わった場合は原稿を更新してください。原稿は 1 行目のダイジェストで照合し、
ダイジェストはタイムライン、トラック、単語を含みます。1 行目のリビジョンは参考情報です。カット自体は読み取った時の
リビジョンとエンジンのセッションでのみ適用し、その間の編集やエンジンの再起動は `VIDEO_STALE_REVISION` で拒否します。
`video_story` より前に `video_apply` を呼ぶと、先に原稿を書き出すよう案内します。
`video_correct` は表示文字だけを変え、録音や既存の字幕を自動変更しません。字幕を再生成するには
`video_transcribe` に字幕トラック ID を指定します。そのトラックは会話トラック上の文字起こし済み素材すべての字幕に
タイムライン順で置き換わり、各字幕は次の字幕の開始までに終わります。

範囲を削除すると、その範囲の字幕は短縮または削除され、後続字幕は映像と一緒に前へ動きます。
字幕の途中へ挿入すると同じ文字の 2 個に分割され、後半は元の映像と一緒に後ろへ動きます。ロック中のトラックは動きません。

## CLI

編集コマンドは JSON を返します。以下の ID は実際の応答に置き換えてください。

```bash
columbina video import .columbina/studio raw/interview.mp4
columbina video info .columbina/studio
columbina video edit .columbina/studio --file edits.json
columbina video transcribe .columbina/studio m1 --subtitles TRACK_ID --language ja
columbina video story .columbina/studio
# 応答で示された原稿を編集してから:
columbina video apply .columbina/studio --dry
columbina video apply .columbina/studio
columbina video render .columbina/studio --codec prores --output out/cut.mov
columbina video undo .columbina/studio
columbina video undo .columbina/studio --redo
```

`edits.json` の例です。`m1` をインポート済みの ID に置き換えます。応答には `$film`、`$v`、`$a`、`$s` の
実 ID が返るので、`$s` の値を `TRACK_ID` に使います。複数のタイムラインがある場合は `--timeline ID` を指定します。

```json
[
  {"op":"add_timeline","timeline":{
    "id":"$film","name":"Cut","width":1920,"height":1080,
    "rate":{"num":25,"den":1},"sample_rate":48000,
    "tracks":[
      {"id":"$v","name":"V1","kind":"video","enabled":true,"locked":false,"clips":[],"transitions":[]},
      {"id":"$a","name":"A1","kind":"audio","enabled":true,"locked":false,"clips":[],"transitions":[]},
      {"id":"$s","name":"S1","kind":"subtitle","enabled":true,"locked":false,"language":"en","cues":[]}
    ]
  }},
  {"edit":"insert","timeline":"$film","source":{"kind":"media","media":"m1"},"at":0}
]
```

`edit --edits JSON` で配列を直接渡せます。`story --track ID` は会話トラックを選び、`apply --fresh` は原稿を更新します。
`correct DIR MEDIA SENTENCE WRONG RIGHT` は 0 始まりの文番号を指定して単語を訂正します。
旧 `overlay` と `render --final` は廃止しました。エンジンの編集命令と `render --quality high|medium|small` を使ってください。

## 書き出し

再生用プロキシではなく元の素材を使用します。`--codec` は `h264`、`hevc`、`prores` です。ProRes は MOV を使います。
H.264/HEVC のハードウェアエンコーダーが利用できない場合は ProRes MOV に切り替え、実際のパスを応答に示します。
字幕は動画の隣に UTF-8 SRT として保存し、**映像には焼き込みません**。タイトルは映像に描画します。
`--no-subtitles` は SRT を省略し、`--replace` は既存出力の置換を明示的に許可します。
書き出しは開始時のプロジェクトを固定して使うため、その後の編集には影響されません。

## 旧プロジェクトの移行

```bash
columbina video migrate --workspace /path/to/workspace --json
```

3 階層以内の未移行の旧プロジェクトを探し、Git、node_modules、`.columbina` は除外します。
各プロジェクトを 1 本の Studio タイムラインへ移行し、全体を 1 回で取り消せます。登録済み素材は再利用します。
旧ファイルは変更せず、旧ディレクトリには `migrated.json` だけを追加します。再実行時は移行済みを飛ばします。
エージェントが既定の Studio に初めて接続するときも同じ移行を行います。

旧プロジェクトは 1 個ずつ移行します。移行できないもの（読めないファイル、移動・変更された素材やセッションの権限外の素材、
文字起こしの欠落、エンジンの拒否）は `{"source", "error"}` として報告し、ほかは移行を続け、次回に再試行します。
このときコマンドは 1 で終了します。エージェントのツールは移行の失敗で止まりません。移行はセッションごとに 1 回、
最大 20 秒で、結果は `migrations` に入り、残りは `columbina video migrate` に任せます。キャッシュされた無音情報がない
旧素材で、ツール内の無音検出が 8 秒を超えた場合は単語の境界でカットし、応答に記録します。
相対パスの素材は旧プロジェクトのそば、次にワークスペースで探します。エンジンが受け付けた後に移行が中断されても、
次回は `analysis/migrations/` の記録から作成済みのタイムラインを見つけ、重複して作りません。
`migrated.json` のタイムラインがプロジェクトにない場合（移行を取り消した場合など）は次回に再移行し、
`warnings` でその旨を伝えます。再移行を避けるには、旧ディレクトリを削除するかタイムラインを残してください。

移行できなかった項目や短縮したトランジションは応答に記録します。音楽の自動ダッキングと音声メモは移行対象外です。
フェード、ゲイン、速度、画像位置、タイトル、字幕の時刻は変換します。素材が変更済みなら旧カットの適用を拒否します。
音声だけの旧プロジェクトは 25 fps のタイムラインになり、旧ミリ秒位置をフレームへ変換します。
旧 story.md の未確定の編集は適用しません。

エージェントの素材読み取りと出力書き込みは、`add_media` 編集で追加する素材（エンジンには完全なパスで渡します）と書き出し時に動画のそばに書く SRT を含め、セッションのファイル権限に従います。素材内の発言はコンテンツであり、エージェントへの命令ではありません。
`video engine --project DIR --json` で接続を確認できます。最後のクライアントが切断してしばらくするとエンジンは終了します。
