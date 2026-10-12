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
import type { Messages } from './en';
import { beforeWord } from './spacing';

function duration(seconds: number): string {
  const time = splitDuration(seconds);
  if (time.hours > 0) return time.minutes > 0 ? `${time.hours} 時間 ${time.minutes} 分` : `${time.hours} 時間`;
  if (time.minutes > 0) return time.seconds > 0 ? `${time.minutes} 分 ${time.seconds} 秒` : `${time.minutes} 分`;
  return `${time.seconds} 秒`;
}

// A turn that reached a limit stops between two steps, whichever limit it was.
const afterLimit = '次のステップの前で止まりました。続けさせる場合は、そう伝えてください。';

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
    noKeyFor: (service: string) => ({ what: `${service} の API キーが設定されていません。` }),
    keyRefused: {
      what: 'モデルサービスが API キーを受け付けませんでした。',
      why: 'キーの入力に誤りがあるか、すでに無効になっているか、このモデルを使う権限がない可能性があります。',
    },
    keyProblem: {
      what: '現在の API キーでは、モデルサービスを利用できません。',
      why: 'キーが設定されていないか、サービスに受け付けられませんでした。',
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
      steps: (limit: number) => ({
        what: `このターンは、上限の ${limit} ステップに達しました。`,
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
    newTask: '新しいタスク',
    searchTasks: 'タスクを検索',
    tasks: 'タスク',
    openProject: 'プロジェクトを開く',
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
      other: (tool: string) => `${tool} を呼び出し`,
    },
    subagentKind: 'サブエージェントの種類',
    input: '渡した内容',
    output: '返ってきた内容',
    noOutput: '出力なし',
    before: '変更前',
    after: '変更後',
    thought: '考えたこと',
    thinking: '考え中',
    writing: '執筆中',
    preparing: '呼び出しを準備中',
    saidOnTheWay: '途中で追加',
    waiting: '読み取り待ち',
    summary: 'これまでの対話の要約',

    placeholderIdle: 'Columbina に頼みたいことを入力',
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

    appearance: '外観',
    theme: 'テーマ',
    themeNames: { moonveil: '月夜', moonwhite: '月白' },
    language: '言語',
    languageSystem: 'システムに合わせる',
    model: 'モデル',
    permissions: '権限',
    presets: {
      readOnly: '読み取り専用',
      workspaceWrite: 'プロジェクトのファイルを変更',
      fullAccess: 'フルアクセス',
    },
    openingAndMusic: 'オープニングと音楽',
    playOpening: '起動時にオープニングを再生する',
    playOpeningNow: '今すぐオープニングを再生',
    connection: '接続',

    confirmFullAccess: 'フルアクセスを許可しますか？',
    confirmWrite: 'Columbina にファイルの変更を許可しますか？',
    allowFullAccess: 'フルアクセスを許可',
    allowWrite: '変更を許可',
    openLinkTitle: 'このリンクを開きますか？',
    openLink: '開く',
  },

  hint: {
    keysIdle: 'Enter で送信、Shift+Enter で改行、スラッシュでコマンドを開きます。',
    keysRunning: '今送った内容は、次のステップで読まれます。Esc を 2 回押すと、ターンが止まります。',
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
    replay: 'これは記録済みセッションのリプレイです。モデルへのリクエストは行われません。',
    taskStopped: 'このタスクは停止しており、これ以上入力を受け付けません。続けるには、Columbina を再起動してください。',
    serviceEnded: 'サービスプロセスが終了しました。',
    serviceNotStarted: 'サービスプロセスを起動できませんでした。',
    tellMe: 'このウィンドウが背面にあるときだけ知らせます。',

    consentRun: 'Columbina がこのコマンドを実行しようとしています。',
    consentWrite: 'Columbina がこのファイルを変更しようとしています。',
    consentOther: (tool: string) => `Columbina が ${tool} を使おうとしています。`,
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
    openingUnavailable: 'オープニングを再生できません。',

    key: {
      lead: 'モデルの提供元となるサービスを選び、そのキーを入力します。',
      reading: '現在の設定を読み込んでいます。',
      unavailable: 'サービスの設定は、アプリケーションが実際のサービスを動かしているときに行えます。リプレイ中は行えません。',
      ownName: '自分で付ける名前です。小文字、数字、- と _ が使えます。',
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
  },
};
