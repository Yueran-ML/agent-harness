<p align="center">
  <img src="docs/assets/columbina-wings.png" width="100%"
       alt="満月を背に翼を広げて浮かぶ人影と、その下で静かな水面に立つ廃墟。">
</p>
<p align="center">
  <sub>ファンアートです。画像生成 AI で作成しました。<a href="#コロンビーナについて">画像について</a></sub>
</p>

<h1 align="center">Columbina</h1>

<p align="center">
  <strong>Python でゼロから書いた、プラグインベースのエージェントハーネス</strong>
</p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a> · <b>日本語</b>
</p>

<p align="center">
  <a href="https://github.com/Yueran-ML/agent-harness/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/Yueran-ML/agent-harness/actions/workflows/ci.yml/badge.svg"></a>
  <img alt="Python 3.11 と 3.12" src="https://img.shields.io/badge/python-3.11%20%7C%203.12-8b7fd6?labelColor=1b1b2f&logo=python&logoColor=white">
  <img alt="カバレッジ 100%" src="https://img.shields.io/badge/coverage-100%25-8b7fd6?labelColor=1b1b2f">
  <img alt="型チェック pyright strict モード" src="https://img.shields.io/badge/types-pyright%20strict-8b7fd6?labelColor=1b1b2f">
  <img alt="リント ruff" src="https://img.shields.io/badge/lint-ruff-8b7fd6?labelColor=1b1b2f">
  <img alt="ライセンス MIT" src="https://img.shields.io/badge/license-MIT-8b7fd6?labelColor=1b1b2f">
</p>

<p align="center">
  <a href="#コロンビーナについて">名前</a> ·
  <a href="#概要">概要</a> ·
  <a href="#ダウンロードとセットアップ">ダウンロードとセットアップ</a> ·
  <a href="#ドキュメント">ドキュメント</a>
</p>

## コロンビーナについて

このプロジェクトの名前は、『原神』のキャラクター、コロンビーナ（Columbina）に由来します。彼女は、ファデュイの執行官「十一人のファトゥス」の第三位、「少女」として登場します。ほかの執行官と同じく、名前はイタリアの即興喜劇コンメディア・デッラルテの登場人物から取られており、彼女の場合はコロンビーナ（Colombina）です。彼女の物語はナド・クライを舞台とし、月と関わりがあります。ここにある画像のすべてに月が描かれているのはそのためです。2026 年の初めからはプレイアブルキャラクターになっており、法器を使う星 5 の水元素キャラクターです。

画像は、一人のファンが思い描いた彼女の姿です。作者が画像生成 AI の ChatGPT で作成したもので、公式のイラストではありません。各ファイルは生成されたときのまま保存しており、作成方法を示すコンテンツクレデンシャル（Content Credentials）が含まれています。

<p align="center">
  <img src="docs/assets/columbina-ruins.png" width="100%"
       alt="水辺の廃墟に座り、両手の間に小さな光を包んでいる同じ人物。">
</p>

<details>
<summary>ストーリーボード</summary>
<br>
<p align="center">
  <img src="docs/assets/columbina-storyboard.png" width="100%"
       alt="同じ人物を描いた十二の小さな場面。Columbina という名前と、そのためのマークが入っています。">
</p>

この画像の中の文字は、画像の一部です。プログラムの機能の一覧ではありません。

</details>

画像を見て彼女に興味を持った方は、ぜひゲームの中で彼女に会ってみてください。『原神』は基本プレイ無料です。公式サイトは [genshin.hoyoverse.com](https://genshin.hoyoverse.com/) で、中国本土向けの公式サイトは [ys.mihoyo.com](https://ys.mihoyo.com/) です。

『原神』とそのキャラクターは HoYoverse（miHoYo）に帰属します。このプロジェクトはファンによるもので、HoYoverse とは関係がなく、公認も受けていません。画像はソフトウェアの一部ではなく、以下のライセンスの対象外です。詳しくは [`docs/assets`](docs/assets/README.md) をご覧ください。

## 概要

Columbina は、エージェントを言語モデルにつなぎ、指定したディレクトリの中で作業させます。エージェントはファイルの読み取り、検索、変更とコマンドの実行を行い、それぞれの呼び出しに何が許されるかは実行の前に判定されます。Columbina を構成するものはすべて、小さな asyncio カーネルのプラグインです。モデルアダプター、ツールレジストリ、セッションログ、そしてエージェントループ自体もそうです。

**ステータス**：開発の初期です。全十段階のうち八つが完了し、Columbina は PyPI で公開されています。残る二つのうち、ベンチマークと、ほかのプログラム向けのサービスはでき上がっています。`columbina` はターミナルでエージェントと対話し、`columbina run` はタスクを一つ渡して回答を出力します。エージェントは OpenAI Chat Completions プロトコルでモデルと通信し、既定では DeepSeek を使います。モデルの上限を超えた対話は自動的に圧縮され、プログラムが強制終了された後でもセッションを再開できます。スキル、フック、サブエージェント、MCP サーバーのツールで、プログラムに手を加えずにエージェントを拡張できます。Docker があれば、エージェントのコマンドは、作業ディレクトリだけがありネットワークのないコンテナの中で実行できます。

## ダウンロードとセットアップ

Columbina には Python 3.11 以降が必要で、[PyPI](https://pypi.org/project/columbina/) からインストールします。

```bash
pip install columbina
```

[uv](https://docs.astral.sh/uv/) を使う場合は、`uv tool install columbina` でほかのプロジェクトとは別にコマンドをインストールし、後で `uv tool upgrade columbina` で最新にします。Model Context Protocol サーバーのツールにはいくつか追加のパッケージが必要で、`pip install "columbina[mcp]"` で一緒に入ります。

モデルはサービスから提供され、サービスには API キーが必要です。Columbina はその両方を尋ねます。

```bash
columbina setup
```

どのサービスを使うか、どのモデルを使うか、そしてキーを尋ねます。DeepSeek、OpenAI、OpenRouter、Moonshot、Zhipu、SiliconFlow は名前で選べます。同じマシンで動く Ollama と LM Studio も同様です。OpenAI Chat Completions プロトコルに対応するほかのサービスは、アドレスを指定して追加できます。キーは入力しても画面に表示されません。キーはシステムのキーリングに保存されます。Windows の資格情報マネージャーや macOS のキーチェーンがこれに当たります。システムにキーリングがない場合は、`~/.columbina` の中のファイルに保存されます。保管の方法は[ガイド](docs/guide.ja.md#サービスとキー)で説明しています。

キーは環境変数で渡すこともでき、環境変数は保存したキーより優先されます。何も選んでいない場合、Columbina は DeepSeek に接続し、`DEEPSEEK_API_KEY` を読みます。Windows の PowerShell では次のようにします。

```powershell
$env:DEEPSEEK_API_KEY = "your-key"
```

Linux や macOS のシェルでは次のようにします。

```bash
export DEEPSEEK_API_KEY="your-key"
```

このように設定した変数は、そのウィンドウを閉じるまで有効です。キーはリクエストを送るときに読み取られます。プロファイルや保存されたセッションに書き込まれることはなく、エラーメッセージからも、表示や保存の前に必ず取り除かれます。

これらがそろっているかは `columbina doctor` で確かめられます。キーを付けてサービスにモデルの一覧を求め、シェルとサンドボックスを調べて、セッションの妨げになるものを一行ずつ示します。

次のコマンドを実行すると、ターミナルでの対話が始まります。エージェントは、コマンドを実行したディレクトリで作業します。

```bash
columbina
```

コマンドラインでタスクを渡すこともでき、その場合は回答が出力されます。

```bash
columbina run "What does this project do? Answer in three sentences."
```

`--cwd DIR` で、エージェントが作業する別のディレクトリを指定できます。

Columbina 自体を開発するには、リポジトリをクローンし、uv にクローンの環境を作らせます。

```bash
git clone https://github.com/Yueran-ML/agent-harness.git
cd agent-harness
uv sync
```

クローンの中では、`uv run columbina` がそこにあるとおりのプログラムを実行します。変更の確かめ方は[ガイド](docs/guide.ja.md#開発)にあります。

## ドキュメント

- [ガイド](docs/guide.ja.md)では、そのほかのことを説明しています。ターミナルとそのコマンド、長い対話、単一タスクの実行、サービスとキー、権限、スキル、フック、サブエージェント、ツールサーバー、Columbina のアーキテクチャ、テストの方法、進捗です。
- [エンジニアリングレポート](docs/report.md)では、Columbina の組み立て、形を決めた選択、テストの方法、性能を説明しています（[中国語版](docs/report.zh-CN.md)）。
- [ベンチマークの表](docs/bench/README.md)には、DeepSeek の二つのモデルでの結果、部分ごとの比較、費用があります。
- [設計文書](docs/plans/2026-10-09-columbina-design.md)は中国語で書かれています。各段階の計画も同じディレクトリにあります。
- [`docs/research`](docs/research) には、Columbina が手本にしたハーネスから学んだことをまとめています。
- [`docs/protocol`](docs/protocol) には、デスクトップアプリがプログラムとやり取りするメッセージを記述しています。

## ライセンス

コードとドキュメントには MIT ライセンスを適用します。[LICENSE](LICENSE) をご覧ください。
