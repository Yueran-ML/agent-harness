/**
 * What the interface says, in Japanese.
 *
 * The keys are those of the English catalogue, and the compiler holds this
 * one to them. The words are not a translation, sentence for sentence: each
 * says the same thing the way a Japanese product would say it.
 *
 * How the words are written, beside what the English catalogue says of it:
 *
 * - Sentences are in the polite form, and end with a full stop of full width.
 * - Punctuation is of full width.
 * - A blank stands between Japanese and a number or a Latin word, and never
 *   between two Japanese characters.
 * - The terms are those of the Japanese guide: ターン, タスク, ステップ, 承認.
 */
import { splitDuration } from './duration';
import type { Messages, Weekday } from './en';
import { afterWord, beforeWord, between } from './spacing';

function duration(seconds: number): string {
  const time = splitDuration(seconds);
  if (time.hours > 0) return time.minutes > 0 ? `${time.hours} 時間 ${time.minutes} 分` : `${time.hours} 時間`;
  if (time.minutes > 0) return time.seconds > 0 ? `${time.minutes} 分 ${time.seconds} 秒` : `${time.minutes} 分`;
  return `${time.seconds} 秒`;
}

// A turn that reached a limit stops between two steps, whichever limit it was.
const afterLimit = '次のステップの前で止まりました。続けさせる場合は、そう伝えてください。';

// The days of the week, for a task that runs on one of them.
const weekdays = { mon: '月曜日', tue: '火曜日', wed: '水曜日', thu: '木曜日', fri: '金曜日', sat: '土曜日', sun: '日曜日' };

export const ja: Messages = {
  duration,

  tool: {
    running: { what: '実行中' },
    waitingForApproval: { what: '承認待ち' },
    done: { what: '完了' },
    allowedByYou: 'あなたが承認',
    exited: (code: number) => ({ what: `終了コード ${code}` }),
    timedOut: {
      call: { what: 'タイムアウト', why: '制限時間を超えたため、止められました。' },
      command: (seconds: number) => ({
        what: 'タイムアウト',
        why: `${duration(seconds)}経過しても終わらなかったため、コマンドは止められました。`,
      }),
      search: { what: 'タイムアウト', why: '検索に時間がかかりすぎたため、止められました。' },
    },
    declined: { what: 'あなたが却下' },
    refused: {
      rule: { what: 'ルールにより拒否', why: '権限のルールが、この操作を認めていません。' },
      hook: { what: 'フックが停止', why: 'フックがこの呼び出しを通しませんでした。' },
      policy: {
        what: '承認なし',
        why: 'この操作にはあなたの承認が必要ですが、このタスクは誰にも確認しない設定になっています。',
      },
      nobody: { what: '承認なし', why: 'この操作には承認が必要ですが、確認できる相手がいませんでした。' },
      approval: { what: '承認なし', why: 'この操作には承認が必要ですが、承認されませんでした。' },
      readOnly: { what: '書き込みなし', why: 'このタスクはファイルの読み取りしかできないため、何も書き込まれませんでした。' },
      protected: { what: '書き込みなし', why: '.git のような、変更から保護されたフォルダーの中にあります。' },
      outside: {
        what: 'プロジェクトの外',
        why: 'このタスクの権限では、プロジェクトフォルダー内のファイルだけを読み書きできます。',
      },
      notWritable: { what: '書き込みなし', why: 'このタスクには、ファイルを書き込む権限がありません。' },
      rulesUnreadable: {
        what: '未実行',
        why: 'このプロジェクトの権限ルールを読み取れません。直るまで、何も実行されません。',
      },
      rulesNotRead: { what: '未実行', why: '権限ルールの読み込みが、まだ終わっていませんでした。' },
    },
    stopped: {
      turnStopped: { what: '停止', why: '実行の途中で止められました。一部はすでに反映されている可能性があります。' },
      programEnded: {
        what: '中断',
        why: '実行中にプログラムが終了しました。反映されたかどうかは分かりません。',
      },
    },
    notRun: {
      turnStopped: { what: '未実行', why: 'これが始まる前に、ターンが止められました。' },
      programEnded: { what: '未実行', why: 'これが始まる前に、プログラムが終了しました。' },
      questionWithdrawn: {
        what: '未実行',
        why: 'あなたへの確認は、回答の前に取り下げられました。',
      },
    },
    unknownTool: { what: '存在しないツール', why: 'モデルが、存在しないツールを呼び出そうとしました。' },
    wrongArguments: { what: '引数が合わない', why: 'モデルが、このツールでは受け取れない引数を渡しました。' },
    failed: {
      error: { what: '失敗' },
      checkFailed: {
        what: '確認できず',
        why: 'チェックに失敗しました。確認できない呼び出しは実行されず、確認できない結果はモデルに渡されません。',
      },
      sandboxUnavailable: {
        what: '未実行',
        why: 'コマンドを閉じ込めるための仕組みが見つからないため、コマンドは実行されません。',
      },
      commandTooLong: { what: 'コマンドが長すぎる', why: 'シェルが受け取れる長さを超えています。' },
      projectMoved: {
        what: 'フォルダーが移動',
        why: 'プロジェクトフォルダーの指す場所が、このタスクの開始時から変わっています。タスクを始め直してください。',
      },
      notFound: { what: '見つからない', why: 'そのようなファイルやフォルダーはありません。' },
      notAFile: { what: 'ファイルではない', why: 'このパスは、フォルダーか、ファイルではない何かを指しています。' },
      notADirectory: { what: 'フォルダーではない', why: 'フォルダーが必要ですが、このパスはファイルを指しています。' },
      notText: { what: 'テキストではない' },
      tooLarge: { what: 'ファイルが大きすぎる', why: '編集で扱える大きさを超えています。' },
      notRead: {
        what: '先に読まれていない',
        why: 'ファイルはすでに存在し、先に読まれていなかったため、上書きされませんでした。',
      },
      changedSinceRead: {
        what: '読み取り後に変更',
        why: '読み取った後でファイルが変更されたため、上書きされませんでした。',
      },
      noMatch: { what: '該当なし', why: '置き換える対象のテキストが、ファイル内にありません。' },
      ambiguous: {
        what: '複数箇所に該当',
        why: '置き換える対象のテキストが複数あり、どれを指すのか決められません。',
      },
      noChange: { what: '変更なし', why: '新しいテキストが、元のテキストと同じです。' },
      invalidPath: { what: '使えないパス', why: 'このパスの書き方は、ここでは使えません。' },
      invalidPattern: {
        what: '使えないパターン',
        why: '検索パターンは、この書き方のままでは使えません。',
      },
      moved: {
        what: '途中で移動',
        why: 'ファイルが直前の場所になかったため、何も行われませんでした。',
      },
      searchFailed: { what: '検索に失敗' },
    },
    withheld: { what: '結果を差し止め', why: '実行されましたが、フックがその結果をモデルに渡しませんでした。' },
    noResult: { what: '結果なし', why: '結果が記録される前に、ターンが終わりました。' },
  },

  model: {
    noKey: { what: 'モデルサービスの API キーが設定されていません。' },
    keyRefused: {
      what: 'モデルサービスが API キーを受け付けませんでした。',
      why: 'キーの入力に誤りがあるか、すでに無効になっているか、このモデルを使う権限がない可能性があります。',
    },
    keyProblem: {
      what: '現在の API キーでは、モデルサービスを利用できません。',
      why: 'キーが設定されていないか、サービスに受け付けられませんでした。',
    },
    named: {
      noKey: (service: string) => ({ what: `${service} の API キーが設定されていません。` }),
      keyRefused: (service: string) => ({
        what: `${service} が API キーを受け付けませんでした。`,
        why: 'キーの入力に誤りがあるか、すでに無効になっているか、このモデルを使う権限がない可能性があります。',
      }),
      keyProblem: (service: string) => ({
        what: `現在の API キーでは、${service} を利用できません。`,
        why: 'キーが設定されていないか、サービスに受け付けられませんでした。',
      }),
    },
    quota: { what: 'モデルサービスのアカウントに残高がありません。' },
    rateLimit: {
      what: 'モデルサービスが混み合っています。',
      why: '現在受け付けられる数を超えるリクエストが送られています。',
    },
    contextTooLong: { what: '対話が長くなりすぎて、モデルが受け取れません。' },
    contentFilter: { what: 'モデルサービスが回答を差し止めました。', why: 'サービスのコンテンツフィルターが回答を止めました。' },
    invalidRequest: {
      what: 'モデルサービスがリクエストを受け付けませんでした。',
      why: 'モデルの名前か、サービスのアドレスが間違っている可能性があります。',
    },
    thinkingRefused: {
      what: 'モデルサービスが、この対話でモデルが以前に考えた内容を受け付けませんでした。',
      why: 'モデルが考えた後で対話の冒頭が変わり、サービスは元の冒頭と合う思考しか受け付けません。'
        + '新しい対話を始めて続けてください。',
    },
    server: { what: 'モデルサービスでエラーが起きました。', why: '原因はサービス側にあります。' },
    timeout: { what: 'モデルサービスの応答に時間がかかりすぎました。' },
    unreachable: {
      what: 'モデルサービスに接続できませんでした。',
      why: 'ネットワークが切れているか、サービスのアドレスが間違っている可能性があります。',
    },
    cutOff: { what: 'モデルが回答を書いている途中で、接続が切れました。' },
    malformed: { what: 'モデルサービスから、読み取れない応答が返されました。' },
    empty: { what: 'モデルから空の回答が返されました。' },
    internal: { what: '回答の受信中に、Columbina の内部で問題が起きました。' },
    unknown: { what: 'モデルへのリクエストに失敗しました。' },
  },

  turn: {
    took: (seconds: number, steps: number) => `${duration(seconds)}、${steps} ステップで完了`,
    stopped: {
      you: { what: 'あなたがこのターンを止めました。' },
      parent: { what: 'このタスクを任せたエージェントが、ターンを止めました。' },
      hook: { what: 'フックがこのターンを止めました。' },
      budget: { what: 'このターンは、設定された上限で止められました。' },
      closing: { what: 'タスクを閉じたときに、このターンは止められました。' },
      unknown: { what: 'このターンは止められました。' },
    },
    limit: {
      steps: (limit: number | null) => ({
        what: limit === null
          ? 'このターンは、ステップ数の上限に達しました。'
          : `このターンは、上限の ${limit} ステップに達しました。`,
        why: afterLimit,
      }),
      tokens: { what: 'このターンは、トークン数の上限に達しました。', why: afterLimit },
      time: { what: 'このターンは、時間の上限に達しました。', why: afterLimit },
    },
    inputStopped: { what: 'フックがあなたの入力を止めました。', why: '入力はモデルに送られていません。' },
    blocked: { what: 'このターンは、続けることが認められませんでした。' },
    crashed: { what: 'Columbina の内部で問題が起き、ターンを続けられませんでした。' },
    outputLimit: {
      what: '回答が途中で切れました。',
      why: 'モデルが、一回の回答で書ける上限まで書きました。',
    },
    interrupted: { what: 'プログラムが終了したため、このターンは途中で終わりました。' },
    forked: { what: 'タスクがコピーされた位置で、このターンは打ち切られました。' },
    unknown: { what: 'このターンは終了しました。' },
  },

  wait: {
    retry: (seconds: number, retry: number) => (seconds > 0
      ? `${duration(seconds)}後に再試行します（${retry} 回目）。`
      : `再試行しています（${retry} 回目）。`),
    retried: (seconds: number, retry: number) => (seconds > 0
      ? `${duration(seconds)}待ってから再試行しました（${retry} 回目）。`
      : `すぐに再試行しました（${retry} 回目）。`),
    gaveUp: '再試行する前に止められました。',
  },

  notice: {
    compactionFailed: { what: 'これまでの対話を要約できませんでした。' },
    compactionInterrupted: {
      what: 'プログラムが終了したため、これまでの対話の要約は途中で終わりました。',
    },
  },

  permissions: {
    readOnlyAsks: 'このプロジェクトのファイルを読み取れます。コマンドを実行する前に確認します。',
    readOnlyStrict: 'このプロジェクトのファイルを読み取れます。ルールで認められたコマンドだけを実行します。',
    writeAsks: 'このプロジェクトのファイルを変更できます。コマンドを実行する前に確認します。',
    writeStrict: 'このプロジェクトのファイルを変更できます。ルールで認められたコマンドだけを実行します。',
    fullAccess: 'このコンピューター上のどのファイルも読み書きでき、確認なしでコマンドを実行します。',
    unknown: 'このタスクに何が許されているかは分かりません。',
    chat: 'ファイルを変更せず、コマンドも実行しません。',
  },

  confinement: {
    none: 'このコンピューターでは、コマンドを閉じ込められません。実行されたコマンドは、あなたにできることを何でも行えます。',
    partial: 'ここでは、コマンドを一部しか閉じ込められません。コマンドがこの権限の範囲を超える可能性があります。',
    full: 'コマンドは、この権限の範囲内に閉じ込められます。',
  },

  actions: {
    retry: '再試行',
    openModelSettings: 'モデルの設定を開く',
    openPermissions: '権限を開く',
    compact: 'これまでの対話を要約する',
  },

  prompt: {
    goOn: 'ターンが途切れたところから続けてください。',
  },

  label: {
    work: {
      spent: (amount: string) => `約 ${amount}`,
      plan: '計画',
      planning: '計画モード',
      exportTask: 'エクスポート',
      right: '頼んだことができた',
      wrong: '頼んだことができなかった',
      rewind: 'ファイルを戻す',
      rewindYes: '戻す',
      keep: 'そのままにする',
      schedules: {
        title: '定期タスク',
        name: '名前',
        folder: 'フォルダー',
        chooseFolder: '選ぶ',
        prompt: 'エージェントに頼むこと',
        every: '頻度',
        kinds: { minutes: '数分ごと', hour: '毎時', day: '毎日', week: '毎週' },
        minutes: '分、5 以上',
        at: '時刻',
        minute: '分',
        days: weekdays,
        weekday: '曜日',
        missed: 'Columbina が閉じている間に過ぎた時刻',
        missedKinds: { run: '開いたときに一度実行する', skip: '飛ばす' },
        add: 'タスクを追加',
        remove: '削除',
        on: '有効',
        when: {
          minutes: (count: number) => `${count} 分ごと`,
          hour: (minute: string) => `毎時 ${minute} 分`,
          day: (time: string) => `毎日 ${time}`,
          week: (day: Weekday, time: string) => `毎週${weekdays[day]} ${time}`,
        },
        last: (when: string, end: string) => `前回 ${when}：${end}`,
      },
      worktrees: {
        title: 'ワークツリー',
        new: 'ワークツリーでタスク',
        name: 'タスクの名前',
        merge: 'マージ',
        remove: '削除',
      },
      groups: {
        title: 'グループ',
        unnamed: '名前なし',
        name: 'グループの名前',
        save: '名前を保存',
        memory: (used: number, most: number) => `記憶：${used} / ${most} 文字`,
        show: '記憶を見る',
        hide: '記憶を閉じる',
        forget: '記憶を消す',
      },
    },

    newTask: '新しいタスク',
    searchTasks: 'タスクを検索',
    tasks: 'タスク',
    openProject: 'プロジェクトを開く',
    balanceAgain: '残高を確認',
    chat: 'チャット',
    justChat: 'チャットだけ',
    settings: '設定',
    hideTasks: 'タスク一覧を隠す',
    showTasks: 'タスク一覧を表示',
    changes: '変更',
    details: '詳細',
    close: '閉じる',
    cancel: 'キャンセル',
    done: '完了',
    apply: '適用',
    copy: 'コピー',
    copied: 'コピーしました',
    send: '送信',
    stop: '停止',
    stopping: '停止中',
    commands: 'コマンド',
    restart: '再起動',
    latest: '最新へ',
    replay: 'リプレイ',
    showLog: 'ログを表示',
    lastWords: '最後の出力',
    recentProjects: '最近のプロジェクト',
    notices: '通知',
    tellMe: 'Columbina が回答を待っているときや、作業を終えたときに知らせる',
    startup: '起動',
    loginItem: 'Windows にサインインしたときに Columbina をトレイで起動する',

    key: {
      title: 'モデルサービス',
      service: 'サービス',
      ownService: '自分で追加するサービス',
      name: '名前',
      address: 'アドレス',
      model: 'モデル',
      contextWindow: 'コンテキストウィンドウ',
      tokens: 'トークン',
      apiKey: 'API キー',
      store: '保存',
      storing: '保存中',
      check: '確認',
      checking: '確認中',
      getKey: 'キーを取得',
      forget: '保存したキーを削除',
      forgetConfirm: '削除する',
      keep: '残す',
      setUp: 'サービスまたはキーを設定',
      inUse: '使用中',
      from: {
        environment: '環境変数のキー',
        stored: 'キー保存済み',
        storedElsewhere: '別のアドレス用のキー',
        none: 'キーなし',
        notNeeded: 'キー不要',
      },
    },

    models: {
      button: (model: string) => `モデル：${model}`,
      filter: '絞り込み、またはモデル名を入力',
      again: 'モデルの一覧を再取得',
      use: (model: string) => `${model} を使用`,
      tryAgain: 'もう一度試す',
      settings: 'モデルの設定',
      services: 'サービス',
      inUse: (service: string) => `${service}（使用中）`,
    },

    effort: {
      title: '思考の深さ',
      button: (level: string) => `思考：${level}`,
      names: { default: '既定', off: 'オフ', low: '低', high: '高', max: '最高', ultra: 'Ultra 全力' },
    },

    attachments: {
      attach: 'ファイルを添付',
      list: 'このメッセージに付けるファイル',
      remove: (name: string) => `${name} を外す`,
      heading: '添付ファイル：',
    },

    github: {
      signIn: 'GitHub でサインイン',
      enterCode: 'このコードを GitHub で入力',
      copyCode: 'コードをコピー',
      openGitHub: 'GitHub を開く',
      left: (clock: string) => `残り ${clock}`,
      newCode: '新しいコードを取得',
      tryAgain: 'もう一度試す',
      checkAgain: 'もう一度確認',
      repositories: 'Columbina がアクセスできるリポジトリ',
      chooseRepositories: 'リポジトリを選ぶ',
      manageRepositories: 'リポジトリを管理',
      signOut: 'サインアウト',
      stay: 'サインインしたままにする',
      revoke: 'GitHub で権限を取り消す',
    },

    platforms: { feishu: 'Feishu', lark: 'Lark', wecom: 'WeCom', qq: 'QQ' },

    feishu: {
      title: 'Feishu アシスタント',
      setUp: 'アプリを設定',
      change: 'アプリを変更',
      start: 'アシスタントを起動',
      starting: '起動しています',
      stop: '停止',
      stopping: '停止しています',
      forget: 'アプリを削除',
      forgetConfirm: '削除する',
      keep: '残す',
      console: '開発者コンソールを開く',
      log: 'ログを表示',
      writable: 'アシスタントが自分のフォルダーのファイルを変更できるようにする',
      folder: 'フォルダーを開く',
      checkAgain: 'もう一度確認',
      copyCode: 'コードをコピー',
      pair: (platform: string) => `このコードを${between(platform)}でボットに送ってください`,
      platform: 'プラットフォーム',
      choose: (platform: string) => `${beforeWord(platform)}を選ぶ`,
      appId: 'App ID',
      appSecret: 'App Secret',
      store: '確認して保存',
      storing: '確認しています',
      steps: '開発者コンソールで行うこと',
      next: '次の手順',
    },

    wecom: {
      title: 'WeCom アシスタント',
      setUp: 'ボットを設定',
      change: 'ボットを変更',
      start: 'アシスタントを起動',
      starting: '起動しています',
      stop: '停止',
      stopping: '停止しています',
      forget: 'ボットを削除',
      forgetConfirm: '削除する',
      keep: '残す',
      console: '管理コンソールを開く',
      log: 'ログを表示',
      writable: 'アシスタントが自分のフォルダーのファイルを変更できるようにする',
      folder: 'フォルダーを開く',
      checkAgain: 'もう一度確認',
      copyCode: 'コードをコピー',
      pair: (platform: string) => `このコードを${between(platform)}のボットとの 1 対 1 のチャットで送ってください`,
      botId: 'Bot ID',
      secret: 'Secret',
      store: '確認して保存',
      storing: '確認しています',
      steps: '管理コンソールで行うこと',
      next: '次の手順',
    },

    qq: {
      title: 'QQ アシスタント',
      setUp: 'ボットを設定',
      change: 'ボットを変更',
      start: 'アシスタントを起動',
      starting: '起動しています',
      stop: '停止',
      stopping: '停止しています',
      forget: 'ボットを削除',
      forgetConfirm: '削除する',
      keep: '残す',
      console: 'QQ オープンプラットフォームを開く',
      log: 'ログを表示',
      folder: 'フォルダーを開く',
      checkAgain: 'もう一度確認',
      copyCode: 'コードをコピー',
      pair: (platform: string) => `${beforeWord(platform)}でボットと個別にチャットし、このコードを送ってください`,
      appId: 'AppID',
      secret: 'AppSecret',
      store: '確認して保存',
      storing: '確認しています',
      steps: 'オープンプラットフォームですること',
      next: '次にすること',
    },

    assistantThinking: {
      model: 'モデル',
      effort: '思考の深さ',
      followModel: 'Columbina が使っているモデル',
      keep: '保存',
      levels: { follow: '全体に合わせる', default: 'サービスの既定', off: '考えない', low: '低', high: '高', max: '最大' },
    },

    start: {
      title: 'はじめに',
      model: 'モデルサービス',
      optional: '任意',
      project: 'プロジェクト',
      connect: '接続',
      change: '変更',
      skip: 'スキップ',
      hide: 'はじめにを閉じる',
      show: 'はじめにを開く',
    },

    notice: {
      consent: 'Columbina が承認を求めています',
      finished: 'Columbina の作業が終わりました',
      stopped: 'Columbina が止まりました',
    },
    mute: '音を消す',
    unmute: '音を出す',
    soundOn: '音あり',
    soundOff: '音なし',

    opening: {
      name: 'オープニング',
      loading: 'オープニングを読み込み中',
      play: 'オープニングを再生',
      again: 'もう一度再生',
      skip: 'スキップ',
      anyKey: '任意のキーかクリックでスキップ',
      begin: '任意のキーで始める',
    },

    music: {
      none: '音楽は未選択',
      choose: 'ファイルを選ぶ',
      change: '別のファイルを選ぶ',
      play: '再生',
      useKept: 'アプリケーションフォルダーの音楽を使う',
      remove: '削除',
      stands: {
        loading: '読み込み中',
        none: '音楽は未選択',
        unavailable: '再生できません',
        blocked: '再生ボタンで開始',
        muted: '消音中',
        playing: '再生中',
      },
    },
    otherProject: '別のプロジェクトを開く',
    skipToInput: '入力欄へ移動',
    confirmLeave: '進行中のターンを止めますか？',
    stay: 'ここに残る',
    stopAndLeave: '止めて離れる',

    ago: {
      now: 'たった今',
      minutes: (count: number) => `${count} 分前`,
      hours: (count: number) => `${count} 時間前`,
      days: (count: number) => `${count} 日前`,
    },

    status: {
      ready: '待機中',
      working: '作業中',
      stopping: '停止中',
      stopped: '停止済み',
      offline: '未接続',
      starting: '起動中',
      opening: '読み込み中',
    },
    context: (percent: number) => `コンテキスト ${percent}%`,
    contextWindow: 'コンテキスト',
    usage: 'このタスクのトークン',
    usageIn: '入力',
    usageOut: '出力',
    usageCached: 'キャッシュ',
    stats: {
      turns: (count: number) => `${count} ターン`,
      steps: (count: number) => `${count} ステップ`,
      speed: (rate: string) => `${rate} トークン/秒`,
      // A count that ends in 万 is written close to the word, one in Latin letters with a blank.
      tokens: (count: string) => `${count}${/[万億兆]$/.test(count) ? '' : ' '}トークン`,
      cached: (percent: string) => `キャッシュ ${percent}%`,
    },
    trace: {
      view: { conversation: '会話', trace: 'トレース' },
      views: 'タスクの表示',
      kinds: { input: '入力', thinking: '思考', reply: '応答', tool: 'ツール', notice: 'お知らせ' },
      overview: 'タスクの各ステップ',
      filters: 'トレースに表示するもの',
      onlyWrong: 'エラーのみ',
      search: 'トレースを検索',
      clear: '検索をクリア',
      found: (at: number, count: number) => `${count} 件中 ${at} 件目`,
      noneFound: '見つかりません',
      turn: (number: number) => `ターン ${number}`,
      ends: {
        completed: '完了', aborted: '停止', blocked: 'ブロック', error: 'エラー', 'max-tokens': '長さの上限',
        interrupted: '中断', forked: '分岐', running: '実行中', other: '終了',
      },
      states: { refused: '拒否', cut: '中断', waiting: '承認待ち', running: '実行中' },
      onlyCalls: (count: number) => `ツールを ${count} 回呼び出し`,
      nothingWritten: 'テキストなし',
      tokens: (read: string, written: string) => `入力 ${read}、出力 ${written}`,
      cut: '途中で終了',
      copy: 'コピー',
      copied: 'コピーしました',
      arguments: '引数',
      result: '結果',
      error: 'エラー',
    },
    tools: 'ツール',
    taskId: 'タスク',

    verb: {
      read: '読み取り',
      write: '書き込み',
      edit: '編集',
      list: '一覧',
      search: '検索',
      run: '実行',
      todo: 'やることを記録',
      skill: 'スキルを使用',
      subagent: '委任',
      githubRepos: 'GitHub のリポジトリを一覧',
      githubRead: 'GitHub で読み取り',
      githubChecks: 'チェックを確認',
      githubClone: 'クローン',
      githubPush: 'プッシュ',
      githubPullRequest: 'プルリクエストを作成',
      githubComment: 'コメント',
      other: (tool: string) => `${tool} を呼び出し`,
    },
    subagentKind: 'サブエージェントの種類',
    input: '渡した内容',
    output: '返ってきた内容',
    noOutput: '出力なし',
    showAllLines: (lines: number) => `全 ${lines} 行を表示`,
    before: '変更前',
    after: '変更後',
    thought: '考えたこと',
    thinking: '考え中',
    writing: '執筆中',
    preparing: '呼び出しを準備中',
    saidOnTheWay: '途中で追加',
    steeredAway: 'あなたの言葉で方向を変えました',
    waiting: '読み取り待ち',
    summary: 'これまでの対話の要約',

    placeholderIdle: 'Columbina に頼みたいことを入力',
    placeholderChat: 'Columbina に話しかける',
    placeholderRunning: '次のステップへの補足を入力',
    placeholderNoProject: 'まずプロジェクトを開いてください',
    placeholderStarting: 'Columbina を起動しています',
    placeholderOpening: 'タスクを開いています',
    placeholderStopped: 'このタスクは止まっています',

    consentTitle: 'あなたの承認が必要です',
    allowOnce: '今回だけ許可',
    allowAlways: '許可し、今後は確認しない',
    reject: '許可しない',
    stopTurn: 'ターンを止める',
    whyItAsks: '確認する理由',

    emptyTitle: '今日は何をしましょうか？',
    emptyNoProjectTitle: 'プロジェクトを開いて始めましょう',
    recentTasks: '最近のタスク',
    skills: 'スキル',
    skillGiven: (name: string | null) => (name ? `スキル：${name}` : 'スキル'),
    skillOfTheAgent: 'Columbina が自分で読み込みます',

    appearance: '外観',
    theme: 'テーマ',
    themeNames: { moonveil: '月夜', moonwhite: '月白' },
    language: '言語',
    languageSystem: 'システムに合わせる',
    reasoningLanguage: '推論の言語',
    reasoningLanguages: { interface: '表示言語に合わせる', zh: '中国語', en: '英語', none: '指定しない' },
    typeSize: '文字サイズ',
    typeSizes: { small: '小', standard: '標準', large: '大' },
    model: 'モデル',
    permissions: '権限',
    account: 'GitHub アカウント',
    presets: {
      readOnly: '読み取り専用',
      workspaceWrite: 'プロジェクトのファイルを変更',
      fullAccess: 'フルアクセス',
    },
    openingAndMusic: 'オープニングと音楽',
    playOpening: '起動時にオープニングを再生する',
    playOpeningNow: '今すぐオープニングを再生',
    about: 'このアプリについて',

    updates: {
      title: 'アップデート',
      told: (version: string) => `新バージョン ${version}`,
      toldInFull: (version: string) => `Columbina ${version} が公開されています。ダウンロードページを開きます`,
      open: 'ダウンロードページを開く',
      skip: 'このバージョンをスキップ',
      checking: '起動時と 1 日 1 回、新しいバージョンを確認する',
      now: '今すぐ確認',
    },

    pet: {
      title: 'デスクトップペット',
      show: 'タスクに合わせて動くペットを表示する',
      choose: 'ペット',
      openFolder: '自作ペットのフォルダーを開く',
      builtin: { moon: 'ちいさな月', cat: 'ちいさな猫', star: 'ちいさな星' },
      mine: '自作',
    },

    studio: {
      open: 'スタジオ',
      close: 'スタジオを閉じる',
      pages: 'スタジオのページ',
      page: { media: 'メディア', edit: 'エディット' },
      pool: 'メディアプール',
      import: '読み込む',
      fps: (rate: string) => `${rate} フレーム毎秒`,
      bits: (bits: number) => `${bits} ビット`,
      audioOnly: '音声のみ',
      proxy: {
        queued: 'プロキシ待機中',
        making: (percent: number) => `プロキシを作成中 ${percent}%`,
        ready: 'プロキシ作成済み',
        failed: 'プロキシ作成失敗',
      },
      makeAgain: '再作成',
      nothingLoaded: '未読み込み',
      play: '再生',
      pause: '一時停止',
      back: '前のフレーム',
      forward: '次のフレーム',
      position: '再生位置',
      useProxies: 'プロキシを使う',
      source: { proxy: 'プロキシ', original: 'オリジナル' },
      sound: { on: '音声オン', off: '音声オフ', unavailable: '音声を使用不可', none: '音声なし' },
      retry: '再試行',
    },

    confirmFullAccess: 'フルアクセスを許可しますか？',
    confirmWrite: 'Columbina にファイルの変更を許可しますか？',
    allowFullAccess: 'フルアクセスを許可',
    allowWrite: '変更を許可',
    openLinkTitle: 'このリンクを開きますか？',
    openLink: '開く',
  },

  hint: {
    work: {
      spent: 'Columbina が知っている価格で見積もった、このタスクのリクエストの費用です。実際の金額はサービスの請求に従います。',
      plan: '計画モードでは、エージェントは読んで計画を立てるだけで、ファイルを変えずコマンドも実行しません。もう一度押すと終わります。',
      exportTask: 'この会話を HTML のページにして、Columbina のエクスポート用フォルダーに保存します。',
      rewindAsk: (turn: number) => `プロジェクトのファイルを、ターン ${turn} の前の状態に戻しますか？/rewind back で元に戻せます。`,
      scheduled: (name: string) => `タスク「${name}」を実行しました。`,
      schedules: {
        about: '定期タスクは Columbina が開いている間に、それぞれ別の会話で実行され、自分のフォルダーのファイルだけを変えます。'
          + '実行中は誰にも確認しないので、確認が必要な操作は実行されません。',
        none: 'まだタスクはありません。',
        failed: 'タスクを変更できませんでした。',
      },
      worktrees: {
        about: 'ワークツリーは、プロジェクトを専用のブランチに写したものです。そこでのタスクは、マージするまであなたのファイルを変えません。',
        mergeAsk: (name: string) => `「${name}」の作業をプロジェクトにマージして、ワークツリーを削除しますか？作業で変わったファイルが変わります。`,
        removeAsk: (name: string) => `ワークツリー「${name}」を削除して、作業を捨てますか？元には戻せません。`,
        failed: 'ワークツリーを変更できませんでした。',
      },
      groups: {
        none: 'ボットはまだグループで呼ばれていません。',
        empty: '記憶は空です。',
        forgetAsk: (group: string) => `アシスタントが「${group}」について覚えていることを消しますか？元には戻せません。`,
        failed: 'グループを変更できませんでした。',
      },
    },

    keysIdle: 'Enter で送信、Shift+Enter で改行、スラッシュでコマンドを開きます。',
    keysRunning: (key: string) => `${key}+Enter で送った内容はすぐに、Enter で送った内容は次のステップで読まれます。Esc を 2 回押すと、ターンが止まります。`,
    escAgain: 'もう一度 Esc を押すと、ターンが止まります。',
    returned: '送った内容は読まれなかったため、入力欄に戻しました。',
    confirmLeave: 'Columbina はまだこのタスクの作業中です。離れると、ターンは止まります。',
    requestFailed: 'うまくいきませんでした。',
    commandsWhenIdle: 'コマンドは、Columbina が待機中のときにだけ実行できます。',
    starting: (folder: string) => `${folder} で Columbina を起動しています。`,
    stillStarting: 'まだ起動中です。',
    openingTask: 'タスクを開いています。',
    reading: (read: number, of: number) => `${of} 件のうち ${read} 件を読み込みました。`,
    compacting: 'これまでの対話を要約しています。',
    noTasks: 'このプロジェクトには、まだタスクがありません。',
    noMatch: '一致するタスクはありません。',
    emptyNoProject: 'Columbina が作業するフォルダーを選んでください。',
    justChat: 'どのプロジェクトにも属さない会話です。Columbina はここでファイルを変更せず、コマンドも実行しません。',
    moreSkills: (count: number) => `ほかに ${count} 件あります。/skills ですべて表示できます。`,
    replay: 'これは記録済みセッションのリプレイです。モデルへのリクエストは行われません。',
    stats: (told: {
      turns: number; steps: number; speed: string | null; tokens: string; read: string; cached: string | null; written: string;
    }) => [
      `${told.turns} ターン、${told.steps} ステップです。`,
      told.speed === null ? '' : `モデルは平均で毎秒 ${told.speed} トークンを書きました（リクエストを送ってから答え終わるまでの時間で計算しています）。`,
      `合計 ${told.tokens} トークンです。入力は ${told.read}${told.cached === null ? '' : `（うち ${told.cached} はキャッシュから）`}、出力は ${told.written} です。`,
    ].filter(Boolean).join(''),
    trace: {
      empty: 'このタスクはまだ始まっていません。',
      step: (told: { turn: number; step: number; took: string | null; calls: number; wrong: number; failed: number }) => [
        `ターン ${told.turn} のステップ ${told.step} です。`,
        told.took === null ? 'モデルはまだ応答していません。' : `モデルは ${told.took} かかりました。`,
        told.failed > 0 ? `その前に ${told.failed} 回のリクエストが失敗しました。` : '',
        told.calls > 0 ? `ツールを ${told.calls} 回呼び出しました${told.wrong > 0 ? `（うち ${told.wrong} 回はエラー）` : ''}。` : '',
      ].filter(Boolean).join(''),
    },
    taskStopped: 'このタスクは停止しており、これ以上入力を受け付けません。続けるには、Columbina を再起動してください。',
    serviceEnded: 'サービスプロセスが終了しました。',
    serviceNotStarted: 'サービスプロセスを起動できませんでした。',
    tellMe: 'このウィンドウが背面にあるときだけ知らせます。',
    loginItem: '前回 Columbina を終了したときに Feishu のアシスタントが動いていれば、一緒に起動します。トレイのアイコンをクリックすると'
      + 'ウィンドウが開きます。',

    consentRun: 'Columbina がこのコマンドを実行しようとしています。',
    consentWrite: 'Columbina がこのファイルを変更しようとしています。',
    consentOther: (tool: string) => `Columbina が ${tool} を使おうとしています。`,
    consentGitHub: {
      clone: 'Columbina が GitHub からリポジトリをクローンしようとしています。',
      push: 'Columbina が GitHub にブランチをプッシュしようとしています。',
      pullRequest: 'Columbina が GitHub でプルリクエストを作成しようとしています。',
      comment: 'Columbina が GitHub にコメントを書こうとしています。',
    },
    consentWaiting: 'Columbina はあなたの回答を待っています。',
    consentSent: '回答を送りました。',
    afterNo: '実行されませんでした。別のやり方を Columbina に伝えられます。',
    scope: {
      thisCommand: '「今後は確認しない」は、このタスクの、一字一句同じこのコマンドにだけ、プロジェクトを閉じるまで適用されます。',
      anyCall: (tool: string) => `「今後は確認しない」は、このタスクでの ${tool} のすべての呼び出しに、プロジェクトを閉じるまで適用されます。`,
      onTarget: (tool: string, target: string) =>
        `「今後は確認しない」は、このタスクでの ${target} に対する ${tool} の呼び出しに、プロジェクトを閉じるまで適用されます。`,
      other: '「今後は確認しない」は、このタスクでの同じ呼び出しに、プロジェクトを閉じるまで適用されます。',
    },

    settingsWhenIdle: 'モデルと権限は、Columbina が待機中のときに変更できます。',
    reasoningLanguage: '次のターンから、すべての対話でモデルはこの言語で推論します。',
    confirmFullAccess: 'Columbina は確認なしでコマンドを実行するようになり、コマンドを制限するものは何もなくなります。',
    confirmWrite: 'Columbina がこのプロジェクトのファイルを変更できるようになります。コマンドを実行する前の確認は、これまでどおり行われます。',
    openLink: '対話の中に書かれていたリンクです。ブラウザーで開きます。',
    commandsList: 'コマンドは Columbina 自身が実行します。モデルには見えません。',
    connectedService: 'このフォルダーで動いているサービスプロセスに接続しています。',
    contextOf: (used: string, window: string) =>
      `直近のリクエストで、${beforeWord(window)}トークンのうち ${beforeWord(used)}を使いました。`,
    contextUnknown: (used: string) => `直近のリクエストで ${beforeWord(used)}トークンを使いました。`,
    noToolsYet: 'ツールは、最初のリクエストを送った後に分かります。',
    noChanges: 'このタスクでは、ファイルは変更されていません。',
    about: (version: string, protocol: number) => `Columbina ${version}、プロトコル ${protocol}。`,
    logKept: 'サービスプロセスで起きた問題は、このコンピューター上のログに書き込まれます。',

    updates: {
      version: (version: string) => `デスクトップ版 ${version}。`,
      fromSource: 'ソースから実行しているときは、新しいバージョンを確認しません。',
      asking: 'GitHub に最新バージョンを問い合わせています。',
      newer: (version: string) => `Columbina ${version} が公開されています。`,
      newest: '最新バージョンです。',
      failed: {
        network: 'GitHub に接続できませんでした。',
        busy: 'GitHub が今は問い合わせを受け付けていません。しばらくしてからもう一度お試しください。',
        unreadable: 'GitHub からリリース一覧を取得できませんでした。',
      },
      sent: '確認では GitHub に Columbina のリリース一覧を問い合わせるだけで、ほかには何も送りません。',
    },
    pet: {
      about: 'ペットはほかのウィンドウの上に表示されます。クリックすると Columbina に戻り、好きな場所へドラッグできます。',
      stateFile: (file: string) => `Columbina がここで作業している間、タスクの状態とツール名を ${file} に書き込みます。自作のペットのプログラムがこれに合わせて動けます。会話の内容は書き込みません。`,
      own: '自作ペットは、pet.json とフレームを並べた画像を入れたフォルダーです。並べ方は Codex のペットと同じです。このフォルダーに入れると、ここで選べます。',
      none: '表示できるペットがありません。',
    },
    openingUnavailable: 'オープニングを再生できません。',

    key: {
      lead: 'モデルの提供元となるサービスを選び、そのキーを入力します。',
      reading: '現在の設定を読み込んでいます。',
      unavailable: 'サービスの設定は、アプリケーションが実際のサービスを動かしているときに行えます。リプレイ中は行えません。',
      ownName: '自分で付ける名前です。小文字で始め、続けて小文字、数字、- と _ が使えます。32 文字までです。',
      ownAddress: 'サービスのアドレスです。例えば https://api.example.com/v1 のように書きます。',
      modelName: 'サービス側での、モデルの名前です。',
      windowSize: 'モデルが受け取れるトークン数です。空欄でもかまいません。',
      notNeeded: 'このサービスにキーは不要です。',
      fromVariable: (variable: string) =>
        `このサービスのキーは、環境変数 ${variable} から読まれます。保存したキーよりも、こちらが優先されます。`,
      stored: 'このサービスのキーは保存済みです。そのまま使う場合は、空欄にしてください。',
      storedElsewhere: 'このサービスのキーは保存されていますが、現在とは別のアドレス用です。このアドレスには送られません。',
      none: 'このサービスのキーは、まだありません。',
      whereItGoes: 'キーはこのウィンドウから Columbina に直接渡され、システムのキーリングに保存されます。'
        + '対話を表示するウィンドウにキーが渡ることはありません。',
      done: (service: string, model: string) => `Columbina は今後 ${service} を使います。モデルは ${model} です。`,
      inKeyring: 'キーはシステムのキーリングに保存されました。',
      inFile: 'キーを受け取るキーリングがありませんでした。キーは Columbina のフォルダー内のファイルに、暗号化されずそのまま保存されています。',
      restart: '保存した内容は、Columbina を再起動した後に使われます。',
      atOnce: 'Columbina は要求のたびにキーを読み込むため、再起動は必要ありません。',
      reached: (service: string, count: number) => `${service} に接続でき、${count} 個のモデルがあります。`,
      noModels: (service: string) => `${service} に接続できましたが、モデルは一つもありません。`,
      notListed: (model: string) => `${model} は、このサービスのモデルの一覧にありません。`,
      likeAKey: 'これはモデル名ではなく、キーのように見えます。',
      blankInKey: 'キーの途中に空白は入りません。貼り付けたときに余分なものが入っていないか確かめてください。',
      failed: {
        network: 'サービスに接続できません。このコンピューターがネットワークにつながっているか、プロキシを使っていればそれが動いているかを確かめてください。',
        timeout: 'サービスが時間内に応答しませんでした。少し待ってから、もう一度試してください。',
        key: 'サービスがこのキーを受け付けません。全体がコピーされているか確かめてください。',
        forbidden: 'サービスはキーを受け付けましたが、この使い方を許可していません。アカウントの残高がないか、キーにモデルを使う権限がない可能性があります。',
        address: 'このアドレスでは、モデルを提供するサービスが応答していません。アドレスが正しいか確かめてください。',
        busy: 'サービスが混み合っています。少し待ってから、もう一度試してください。',
        unknown: '確認できませんでした。',
      },
      notRunning: {
        ollama: 'このコンピューターの Ollama が応答しません。ollama serve などで起動してから、もう一度確認してください。',
        lmstudio: 'このコンピューターの LM Studio が応答しません。ローカルサーバーを起動してから、もう一度確認してください。',
        other: 'このコンピューターのサービスが応答しません。起動してから、もう一度確認してください。',
      },
      forgetAsk: (service: string) => `${service} に保存したキーを削除しますか？ほかの設定はそのまま残ります。`,
      forgotten: (service: string) => `${service} のキーを削除しました。`,
      noneToForget: (service: string) => `${service} にはキーが保存されていませんでした。`,
    },

    effort: {
      notes: {
        default: 'サービスに任せます。',
        off: '考えずに、最も速く答えます。',
        low: '少しだけ考えます。',
        high: 'タスクをじっくり考えます。',
        max: 'モデルができる限り深く考えます。',
        ultra: '最も深く考え、サブエージェントが作業を分担して最後に見直します。最も遅く、費用も最もかかります。この実行の間だけ有効です。',
        ultraTeam: 'サブエージェントが作業を分担して最後に見直します。サービスにより深く考えるよう求めることはしません。この実行の間だけ有効です。',
      },
      whenIdle: '思考の深さは、Columbina が待機中のときに変更できます。',
      notKept: '思考の深さは切り替わりましたが、次回の起動用に保存できませんでした。',
    },

    models: {
      asking: (service: string) => `${service} からモデルの一覧を取得しています。`,
      none: (service: string) => `${service} にはモデルが一つもありません。`,
      noMatch: '一致するモデルはありません。',
      whenIdle: 'モデルは、Columbina が待機中のときに変更できます。',
      unavailable: 'モデルの一覧は、アプリケーションが実際のサービスを動かしているときに表示されます。リプレイ中は表示されません。',
      notKept: 'モデルは切り替わりましたが、次回の起動用に保存できませんでした。',
      switch: (service: string) => `${service} のモデルを選ぶと、Columbina は ${service} で起動し直し、今のタスクを続けます。`,
      notSwitched: 'サービスは切り替わりませんでした。',
      failed: {
        network: 'サービスに接続できません。このコンピューターがネットワークにつながっているか、プロキシを使っていればそれが動いているかを確かめてください。',
        timeout: 'サービスが時間内に応答しませんでした。少し待ってから、もう一度試してください。',
        key: 'サービスが、設定されているキーを受け付けません。',
        forbidden: 'サービスはキーを受け付けましたが、モデルの一覧の取得を許可していません。アカウントの残高がないか、キーにモデルを使う権限がない可能性があります。',
        address: 'このサービスのアドレスでは、モデルを提供するサービスが応答していません。',
        busy: 'サービスが混み合っています。少し待ってから、もう一度試してください。',
        unknown: 'モデルの一覧を取得できませんでした。',
      },
      notRunning: {
        ollama: 'このコンピューターの Ollama が応答しません。ollama serve などで起動してから、もう一度試してください。',
        lmstudio: 'このコンピューターの LM Studio が応答しません。ローカルサーバーを起動してから、もう一度試してください。',
        other: 'このコンピューターのサービスが応答しません。起動してから、もう一度試してください。',
      },
    },

    attachments: {
      picture: 'モデルは read ツールで画像を見ます（使うサービスが画像に対応している場合）。',
      refused: {
        gone: (name: string) => `${name} は見つからなくなりました。`,
        notAFile: (name: string) => `${name} はフォルダーです。添付できるのはファイルだけです。`,
        tooBig: (name: string) => `${name} は 25 MB を超えています。`,
        tooMany: (name: string) => `${name} は添付されませんでした。一つのメッセージに付けられるファイルは 10 個までです。`,
      },
      command: 'コマンドにはファイルを付けられません。ファイルは次のメッセージに付きます。',
      drop: 'ここで離すと、ファイルを添付します。',
      notKept: 'ファイルをプロジェクトに保存できなかったため、メッセージは送られませんでした。',
    },

    start: {
      lead: 'あと少しの手順で、Columbina が作業を始められます。',
      noKey: (service: string) => `${service} のキーがまだ設定されていません。`,
      github: 'サインインすると、Columbina があなたのリポジトリ、issue、プルリクエストを扱えるようになります。'
        + 'あとで設定からサインインすることもできます。',
    },

    github: {
      lead: 'サインインすると、Columbina はあなたの名前でリポジトリ、issue、プルリクエスト、チェックを読み、'
        + 'クローン、プッシュ、プルリクエストの作成、コメントができるようになります。'
        + 'プッシュ、プルリクエスト、コメントの前には毎回確認します。フルアクセスか規則で許可されている場合は確認しません。',
      checking: 'GitHub にサインインしている人を確認しています。',
      asking: 'GitHub にコードを求めています。',
      waiting: 'ブラウザーで確認されるまで、Columbina は待っています。',
      expired: 'コードの有効期限が切れました。',
      signedIn: (login: string) => `@${login} として GitHub にサインインしました。`,
      noRepositories: 'Columbina がアクセスできるのは、あなたが GitHub で選んだリポジトリだけです。まだ一つも選ばれていません。',
      moreRepositories: (count: number) => `ほかに ${count} 件あります。`,
      offline: 'Columbina がアクセスできるリポジトリを GitHub に問い合わせられませんでした。',
      ended: 'GitHub のサインインの有効期限が切れました。もう一度サインインしてください。',
      unavailable: 'GitHub へのサインインは、アプリがサービスを自分で動かしているときにできます。再生中はできません。',
      unreadable: 'このコンピューターに保存された GitHub のサインイン情報を読み取れません。',
      statusFailed: 'Columbina は GitHub にサインインしている人を確認できませんでした。',
      signOutAsk: (login: string) =>
        `このコンピューターで GitHub からサインアウトしますか？その後、Columbina は @${login} の名前で操作できなくなります。`,
      signedOut: 'このコンピューターでサインアウトしました。アプリに与えた権限は、GitHub で取り消すまで有効です。',
      signOutFailed: 'サインアウトできませんでした。',
      failed: {
        unreachable: 'GitHub に接続できません。このコンピューターがネットワークにつながっているか、プロキシを使っていればそれが動いているかを確かめてください。',
        timeout: 'GitHub が時間内に応答しませんでした。少し待ってから、もう一度試してください。',
        busy: 'GitHub が混み合っています。少し待ってから、もう一度試してください。',
        expired: '確認される前にコードの有効期限が切れました。新しいコードを取得してください。',
        denied: 'GitHub でサインインが拒否されました。',
        rateLimited: 'GitHub から、次のリクエストまで待つよう求められています。待つ時間は詳細に書かれています。',
        unregistered: 'この Columbina では、まだ GitHub にサインインできません。',
        store: 'サインインの情報をこのコンピューターに保存できませんでした。',
        ended: 'サインインが完了する前に止まりました。',
        notStarted: 'サインインを始められませんでした。',
        unknown: 'サインインできませんでした。',
      },
    },

    feishu: {
      lead: 'Feishu や Lark で Columbina と話せます。オープンプラットフォームで作ったあなた自身のアプリを使います。'
        + 'アシスタントはあなたにだけ返信し、アプリに共有されたドキュメントを読みます。コマンドは実行せず、あなたのファイルも変更しません。'
        + '送ったファイルとスキルは、アシスタント専用のフォルダーに保存されます。',
      reading: 'Feishu アシスタントの状態を確認しています。',
      unavailable: 'Feishu アシスタントの設定は、アプリケーションが実際のサービスを動かしているときに行えます。リプレイ中は行えません。',
      statusFailed: 'Columbina は Feishu アシスタントの状態を確認できませんでした。',
      notRunning: 'アシスタントは動いていません。',
      connecting: (platform: string) => `${beforeWord(platform)}に接続しています。`,
      reconnecting: (platform: string) => `いまは${between(platform)}に接続できません。アシスタントは接続を試し続けます。`,
      connected: (platform: string) => `接続しました。アシスタントは${between(platform)}であなたに返信します。`,
      trouble: '最近、うまくいかなかったことがあります。',
      pairNote: '最初にコードを送った人が、アシスタントの返信する相手になります。コードの有効期限は 10 分です。'
        + '期限が切れたあとにボットへメッセージを送ると、ここに新しいコードが表示されます。',
      pairNotHeard: 'ボットから返信がない場合は、開発者コンソールで、受信方法が長時間接続であること、'
        + 'メッセージ受信のイベントが追加されていること、アプリのバージョンがリリースされていることを確かめてください。',
      noCode: '間違ったコードが多すぎたため、コードは無効になりました。アシスタントを停止してもう一度起動すると、新しいコードが表示されます。',
      stopping: 'アシスタントを停止しています。',
      elsewhere: 'アシスタントはこのコンピューターの別の場所（ターミナルなど）で動いています。停止はそちらで行ってください。',
      withColumbina: 'アシスタントが動いている間は、ウィンドウを閉じても Columbina はトレイに残り、アシスタントは返事を続けます。'
        + '両方を止めるにはトレイから終了してください。終了したときに動いていれば、次に開いたときにまた起動します。',
      writable: 'オンにすると、アシスタントは自分のフォルダーでファイルを作成・変更できます。コマンドは引き続き実行しません。'
        + '動いているアシスタントは、変更のために起動し直します。',
      writableFailed: 'アシスタントがファイルを変更できるかどうかを切り替えられませんでした。',
      folderFailed: 'アシスタントのフォルダーを開けませんでした。',
      service: 'アシスタントは、起動したときに Columbina が使っているサービスと話します。別のサービスを選んだあとは、'
        + 'アシスタントを停止して、もう一度起動してください。',
      ended: 'アシスタントが止まりました。',
      lastError: '前回のアシスタントの実行は正常に終わりませんでした。',
      startFailed: 'アシスタントを起動できませんでした。',
      stopFailed: 'アシスタントを停止できませんでした。',
      forgetAsk: (appId: string) => `このコンピューターからアプリ ${appId} を削除しますか？App Secret と返信相手の情報が削除され、`
        + 'アシスタントが動いていれば先に停止します。オープンプラットフォーム上のアプリはそのまま残ります。',
      forgotten: 'このコンピューターからアプリを削除しました。',
      forgetFailed: 'アプリを削除できませんでした。',
      windowLead: 'Feishu または Lark のオープンプラットフォームで作ったアプリです。App ID と App Secret はアプリの認証情報のページにあります。',
      appId: 'cli_ で始まります。',
      appIdWrong: 'App ID は cli_ で始まり、そのあとに英字と数字が続きます。',
      secret: 'App Secret はこのウィンドウから Columbina に直接渡され、システムのキーリングに保存されます。'
        + '会話を表示するウィンドウが受け取ることはありません。',
      secretAgain: '保存された App Secret は表示されません。アプリを保存し直すには、もう一度入力してください。',
      blankInSecret: 'App Secret の途中に空白は入りません。貼り付けたときに余分なものが入っていないか確かめてください。',
      wrongPlatform: (asked: string, other: string) => `${beforeWord(asked)}にはこのアプリがありません。`
        + `${beforeWord(other)}の開発者コンソールで作ったアプリなら、プラットフォームで${between(other)}を選んで、もう一度保存してください。`,
      done: (appId: string, platform: string) => `${beforeWord(platform)}がアプリ ${appId} を受け付け、保存しました。`,
      inKeyring: 'App Secret はシステムのキーリングに保存されました。',
      inFile: 'App Secret を受け取るキーリングがありませんでした。Columbina のフォルダー内のファイルに、暗号化されずそのまま保存されています。',
      steps: (platform: string) => [
        '開発者コンソールでカスタムアプリを作成します。App ID と App Secret は認証情報と基本情報にあります。',
        '機能の追加でボットを追加します。',
        '権限管理で下の権限を追加します。',
        'ここでアプリを保存したら、設定でアシスタントを起動します。',
        'アシスタントの実行中に、イベントとコールバックで受信方法を長時間接続に設定し、メッセージ受信のイベント'
          + '（im.message.receive_v1）を追加します。',
        'バージョン管理とリリースでバージョンを作成してリリースを申請します。利用できる人の範囲に自分を含めてください。',
        `${beforeWord(platform)}でボットの名前を検索し、1 対 1 のチャットでコードを送ります。`,
      ],
      scopes: '最後の 3 つはドキュメントを読むためのものです。不要なら追加しなくてもかまいません。',
    },

    balance: {
      reading: 'サービスにアカウントの残高を問い合わせています。',
      amount: (service: string, total: string, paid: string, given: string) =>
        `${beforeWord(service)}の残高は ${total} です（入金 ${paid}、付与 ${given}）。`,
      none: (service: string) => `${beforeWord(service)}はアカウントの残高を示していません。`,
      cannotPay: '残高が足りず、モデルを呼び出せません。サービスのページでチャージしてください。',
      unsupported: (service: string) => `${beforeWord(service)}はプログラムに残高を教えません。サービスのページで確認してください。`,
      failed: (service: string) => `${beforeWord(service)}に残高を問い合わせることができませんでした。`,
    },

    wecom: {
      lead: 'WeCom で Columbina と話せます。あなた自身が作ったスマートボットを使います。'
        + 'アシスタントはボットとの 1 対 1 のチャットで、あなたにだけ返信します。コマンドは実行せず、あなたのファイルも変更しません。'
        + '送ったファイルとスキルは、アシスタント専用のフォルダーに保存されます。'
        + 'グループでは、ボットをメンションした人に誰でも返信し、グループごとに別の会話になります。'
        + 'グループでは読むだけで、あなたのファイル、スキル、個人的な指示は見えません。',
      reading: 'WeCom アシスタントの状態を確認しています。',
      unavailable: 'WeCom アシスタントの設定は、アプリケーションが実際のサービスを動かしているときに行えます。リプレイ中は行えません。',
      statusFailed: 'Columbina は WeCom アシスタントの状態を確認できませんでした。',
      notRunning: 'アシスタントは動いていません。',
      connecting: (platform: string) => `${beforeWord(platform)}に接続しています。`,
      reconnecting: (platform: string) => `いまは${between(platform)}に接続できません。アシスタントは接続を試し続けます。`,
      connected: (platform: string) => `接続しました。アシスタントは${between(platform)}であなたに返信します。`,
      trouble: '最近、うまくいかなかったことがあります。',
      pairNote: '最初にコードを送った人が、アシスタントの返信する相手になります。コードの有効期限は 10 分です。'
        + '期限が切れたあとにボットへメッセージを送ると、ここに新しいコードが表示されます。',
      pairNotHeard: 'ボットから返信がない場合は、管理コンソールで、ボットが API モードの長時間接続を使っていること'
        + '（コールバック URL ではないこと）を確かめてください。',
      noCode: '間違ったコードが多すぎたため、コードは無効になりました。アシスタントを停止してもう一度起動すると、新しいコードが表示されます。',
      stopping: 'アシスタントを停止しています。',
      elsewhere: 'アシスタントはこのコンピューターの別の場所（ターミナルなど）で動いています。停止はそちらで行ってください。',
      withColumbina: 'アシスタントが動いている間は、ウィンドウを閉じても Columbina はトレイに残り、アシスタントは返事を続けます。'
        + '両方を止めるにはトレイから終了してください。終了したときに動いていれば、次に開いたときにまた起動します。',
      writable: 'オンにすると、アシスタントは自分のフォルダーでファイルを作成・変更できます。コマンドは引き続き実行しません。'
        + '動いているアシスタントは、変更のために起動し直します。',
      writableFailed: 'アシスタントがファイルを変更できるかどうかを切り替えられませんでした。',
      folderFailed: 'アシスタントのフォルダーを開けませんでした。',
      service: 'アシスタントは、起動したときに Columbina が使っているサービスと話します。別のサービスを選んだあとは、'
        + 'アシスタントを停止して、もう一度起動してください。',
      ended: 'アシスタントが止まりました。',
      lastError: '前回のアシスタントの実行は正常に終わりませんでした。',
      startFailed: 'アシスタントを起動できませんでした。',
      stopFailed: 'アシスタントを停止できませんでした。',
      forgetAsk: (botId: string) => `このコンピューターからボット ${botId} を削除しますか？Secret と返信相手の情報が削除され、`
        + 'アシスタントが動いていれば先に停止します。WeCom 上のボットはそのまま残ります。',
      forgotten: 'このコンピューターからボットを削除しました。',
      forgetFailed: 'ボットを削除できませんでした。',
      windowLead: 'WeCom の管理コンソールで作ったスマートボットです。API モードで長時間接続を選ぶと、ボットのページに Bot ID と Secret が表示されます。',
      botId: 'ボットのページに表示されているとおりに入力します。',
      botIdWrong: 'Bot ID は英字・数字・記号からなり、途中に空白は入りません。',
      secret: 'Secret はこのウィンドウから Columbina に直接渡され、システムのキーリングに保存されます。'
        + '会話を表示するウィンドウが受け取ることはありません。',
      secretAgain: '保存された Secret は表示されません。ボットを保存し直すには、もう一度入力してください。',
      blankInSecret: 'Secret の途中に空白は入りません。貼り付けたときに余分なものが入っていないか確かめてください。',
      done: (botId: string) => `WeCom がボット ${botId} を受け付け、保存しました。`,
      inKeyring: 'Secret はシステムのキーリングに保存されました。',
      inFile: 'Secret を受け取るキーリングがありませんでした。Columbina のフォルダー内のファイルに、暗号化されずそのまま保存されています。',
      steps: [
        'WeCom の管理コンソールでスマートボットを作成します。',
        'ボットのページで API モードを有効にし、長時間接続を選びます。ページに Bot ID と Secret が表示されます。',
        'ここでボットを保存したら、設定でアシスタントを起動します。',
        'WeCom でボットを見つけ、1 対 1 のチャットでコードを送ります。',
      ],
    },

    qq: {
      lead: 'QQ で Columbina とチャットできます。あなた自身が作ったボットを使います。個別のチャットではあなたにだけ返信し、'
        + 'グループに入れるとメンションした人に誰でも返信し、グループごとに別の会話になります。すばやく答え、深くは考えず、'
        + 'ファイルの変更もコマンドの実行もしません。グループではあなたのファイルや個人的な指示は見えません。',
      reading: 'QQ アシスタントの状態を確認しています。',
      unavailable: 'QQ アシスタントは、アプリが自分でサービスを動かしているときにここで設定できます。再生中はできません。',
      statusFailed: 'Columbina は QQ アシスタントの状態を確認できませんでした。',
      notRunning: 'アシスタントは動いていません。',
      connecting: (platform: string) => `${beforeWord(platform)}に接続しています。`,
      reconnecting: (platform: string) => `いまは${between(platform)}に接続できません。アシスタントは接続を試し続けます。`,
      connected: (platform: string) => `接続しました。アシスタントは${between(platform)}であなたに返信します。`,
      trouble: '最近うまくいかなかったことがあります。',
      pairNote: '最初にコードを送った人が、個別のチャットでアシスタントが返信する相手になります。コードは 10 分間有効です。'
        + 'その後はボットに何かメッセージを送ると、ここに新しいコードが出ます。',
      pairNotHeard: 'ボットが答えないときは、オープンプラットフォームでイベントが WebSocket で届く設定になっているか、'
        + '公開前のボットを使えるメンバーにあなたが入っているかを確認してください。',
      noCode: 'コードの入力を何度も間違えたため、コードは無効になりました。アシスタントを停止して起動し直すと、新しいコードが出ます。',
      stopping: 'アシスタントを停止しています。',
      elsewhere: 'アシスタントはこのコンピューターの別の場所（ターミナルなど）で動いています。そこで停止してください。',
      withColumbina: 'アシスタントが動いている間は、ウィンドウを閉じても Columbina はトレイに残り、アシスタントは返信を続けます。'
        + 'トレイから終了すると両方が止まります。Columbina の終了時に動いていれば、次回の起動時にまた動きます。',
      folderFailed: 'アシスタントのフォルダーを開けませんでした。',
      service: 'アシスタントは起動時に Columbina が使っているサービスを使います。サービスを変えたら、アシスタントを停止して起動し直してください。',
      ended: 'アシスタントが止まりました。',
      lastError: 'アシスタントの前回の実行は正常に終わりませんでした。',
      startFailed: 'アシスタントを起動できませんでした。',
      stopFailed: 'アシスタントを停止できませんでした。',
      forgetAsk: (appId: string) => `このコンピューターでボット ${appId} を削除しますか？ AppSecret と返信する相手が削除され、`
        + '動いているアシスタントは先に止まります。QQ のボットはそのまま残ります。',
      forgotten: 'このコンピューターでボットを削除しました。',
      forgetFailed: 'ボットを削除できませんでした。',
      windowLead: 'QQ オープンプラットフォームで作ったボットです。開発設定に AppID と AppSecret が表示されます。',
      appId: 'ボットの開発設定に表示されているとおりに入力します。',
      appIdWrong: 'AppID は文字、数字、記号でできていて、空白は入りません。',
      secret: 'AppSecret はこのウィンドウから Columbina に渡され、システムのキーリングに保存されます。'
        + '会話を表示するウィンドウには渡りません。',
      secretAgain: '保存済みの AppSecret は表示されません。もう一度入力するとボットを保存し直せます。',
      blankInSecret: 'AppSecret に空白は入りません。貼り付けたときに余計なものが入っていないか確認してください。',
      done: (appId: string) => `QQ がボット ${appId} を受け付け、保存しました。`,
      inKeyring: 'AppSecret はシステムのキーリングにあります。',
      inFile: 'AppSecret を受け付けるキーリングがありませんでした。Columbina のフォルダーのファイルに暗号化されずに保存されています。',
      steps: [
        'QQ オープンプラットフォームでボットを作ります。',
        '開発設定で、イベントを WebSocket で受け取るようにします。そこに AppID と AppSecret が表示されます。',
        'ここでボットを保存したら、設定でアシスタントを起動します。',
        'QQ でボットを見つけ、個別のチャットでコードを送ります。グループに入れると、誰でもメンションできます。',
      ],
    },

    assistantThinking: {
      note: '空のままの項目は全体の設定に合わせます。動いているアシスタントは、変更すると起動し直します。',
      failed: 'アシスタントのモデルと思考の深さを変更できませんでした。',
    },

    music: {
      about: '音楽は、オープニングの間も作業中も繰り返し再生されます。',
      kinds: 'MP3、WAV、M4A、OGG、FLAC に対応し、100 MB までです。ファイルはこのコンピューターにだけ保存されます。',
      problem: {
        cannotPlay: 'このファイルは再生できません。別のファイルを選んでください。',
        cannotOpenKept: 'アプリケーションフォルダーの音楽を開けません。別のファイルを選んでください。',
        wrongKind: 'MP3、WAV、M4A、OGG、FLAC のいずれかのファイルを選んでください。',
        wrongSize: '空ではなく、100 MB 以下のファイルを選んでください。',
        noKept: 'アプリケーションフォルダーに音楽がありません。ファイルを選んでください。',
      },
      unkept: {
        cannotRead: '以前の選択を読み取れません。今の選択は、このウィンドウでだけ有効です。',
        thisWindowOnly: 'このファイルは、このウィンドウでだけ再生されます。次回の起動に向けて保存することはできませんでした。',
        notCleared: '音楽はこのウィンドウから削除されました。保存されていたコピーは消せなかったため、'
          + '次回の起動時にまた現れる可能性があります。',
        choiceNotKept: 'アプリケーションフォルダーの音楽は、このウィンドウで再生されます。この選択は保存できませんでした。',
      },
    },

    studio: {
      browser: 'プレビューはデスクトップアプリで表示されます。',
      opening: 'プロジェクトをスタジオで開いています。',
      takingOver: 'エージェントからプロジェクトを引き継いでいます。',
      stopped: 'スタジオのエンジンが停止しました。',
      missing: 'スタジオのエンジンがインストールされていません。',
      notOpened: 'スタジオを開けませんでした。',
      unsupported: 'このシステムでは、まだプレビューを表示できません。',
      headless: 'ウィンドウのないエンジンがプロジェクトを開いているため、プレビューを表示できません。',
      noMedia: 'メディアはまだありません。読み込んで始めてください。',
      load: 'メディアページでメディアをダブルクリックすると、ここに表示されます。',
      inexact: '正確なフレームの準備ができるまで、再生ヘッド付近のフレームを表示します。',
      importFailed: 'メディアを読み込めませんでした。',
      loadFailed: 'このメディアを開けませんでした。',
      failed: 'エンジンが操作を実行できませんでした。',
      proxyFailed: 'プロキシを作成できませんでした。',
      useProxies: 'プロキシは、スクラブしやすいメディアのコピーです。自動で作成されます。',
      source: {
        proxy: '画像をプロキシから読み込んでいます。',
        original: '画像を元のメディアから読み込んでいます。',
      },
      sound: {
        on: '音声を再生しています。',
        off: '音声は標準速度で再生するときだけ出ます。',
        unavailable: '使用できるサウンドデバイスがないため、画像のみ再生します。',
        none: 'このメディアには音声がありません。',
      },
    },
  },
};
