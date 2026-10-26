# 動画の編集

Columbina は、動画の中で話されたことばをもとに映像を切り、その上に映像、音楽、タイトルを重ねられます。使うのはシステムの ffmpeg で、話されたことばの書き起こしには、このマシン上の faster-whisper を使います。どこにも何も送りません。

## 用意するもの

- **ffmpeg 6.0 以降**と、その隣にある ffprobe：Windows では `winget install Gyan.FFmpeg`、macOS では `brew install ffmpeg`、Linux ではディストリビューションの `ffmpeg` パッケージ。
- **faster-whisper とモデル**（書き起こし用）。Columbina の Python に `pip install faster-whisper` で入れ、モデルを自分のフォルダーにダウンロードします：

  ```bash
  huggingface-cli download Systran/faster-whisper-small --local-dir ~/models/faster-whisper-small
  ```

  `large-v3` はより正確で、グラフィックスカードが必要です。`small` はどのプロセッサーでも動きます。Columbina が自分でモデルをダウンロードすることはありません。

## エージェントに任せる

`COLUMBINA_VIDEO` が `1` のときに編集のツールが使えます。`COLUMBINA_WHISPER_MODEL` でモデルのフォルダーを指定します：

```bash
COLUMBINA_VIDEO=1 COLUMBINA_WHISPER_MODEL=~/models/faster-whisper-small columbina
```

あとは、たとえば「`raw/interview.mp4` から `talk` で動画を作って。言いよどみと言い直した文を切り、長い間を短くして、ツールの話をしているところに `raw/screen.mp4` を重ね、全体に `music.mp3` を流して」と頼みます。エージェントがファイルの取り込み、書き起こし、台本の編集、レイヤーの追加、プレビューの書き出しを行います。

## 台本

プロジェクトはひとつのフォルダーです。`project.json` に素材、`timeline.json` にコミットされた映像、`story.md` に話のテキストがあります。台本の 1 行が 1 文です：

```text
<!-- columbina story: 3f9a61c2e0d4 -->

[talk.0 00:01] こんにちは、~~えー~~ 今日は動画の切り方の話です。
[talk.1 00:06] ~~この~~ このツールは文字で切れます。
  > picture screen picture-1
[pause 1.8s]
[talk.2 00:12] ~~まず見てみると、いや、言い直します。~~
```

- 行を消すか `~~` で打ち消すと、その文が切られます。単語を消すか打ち消すと、その単語が切られます。打ち消し線を外すと戻ります。
- 行を動かすと、その文が移ります。
- `[pause 1.8s]` はその文のあとの間です。数を小さくすると短くなり、行を消すとごく短くなります。
- `>` で始まる行は、そこに重なっているものの説明で、読み取り専用です。
- 単語の追加や変更はできません。台本の文字は録音された声そのものです。聞き違いは `video_correct` か `columbina video correct` で表示だけを直します。

台本は `video_apply` でコミットします。結果には、何を切ったか、動画の長さの前後、言葉が切られて置き場所のなくなったレイヤー、間のないところで切ったため聞き取れるかもしれない箇所が書かれます。コミットはどれも取り消せます。

2 つの文の間の切れ目は、その間の無音の中に置かれ、両側に少し無音を残します。書き起こしの時刻は、先に間に合わせて全体で補正されます。

## 手作業で

同じ操作を、モデルなしで：

```bash
columbina video import talk raw/interview.mp4 raw/screen.mp4 music.mp3
columbina video transcribe talk interview --model ~/models/faster-whisper-small
# talk/story.md を編集
columbina video apply talk
columbina video overlay talk add --kind picture --source screen --sentence interview.1 --words "文字で切れます" --position top-right --scale 0.4
columbina video overlay talk add --kind sound --source music --volume 0.4
columbina video render talk            # プレビュー
columbina video render talk --final    # 本番の動画と、その隣の字幕ファイル
columbina video undo talk
columbina video info talk
```

前もって書き起こした単語は、モデルの代わりに `--words FILE` で渡せます。`start_us`、`end_us`、`text` を持つオブジェクトのリストです。

## 安全

素材の読み込みとプロジェクトの書き込みは、ほかのファイルと同じく、セッションのルールに従います。素材に対する ffmpeg の実行はどれも、ローカルのファイルだけ、映像・音声・画像の形式だけを許します。ほかのファイルを指すリスト（HLS や concat のリストなど）は開きません。ffmpeg のフィルターの引数にパスを書くことはありません。素材の中で話されたことばは映像のせりふであり、エージェントへの指示ではありません。
