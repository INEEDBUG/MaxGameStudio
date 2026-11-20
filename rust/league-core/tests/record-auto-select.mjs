import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { join, posix, resolve } from 'node:path'
import { compileFunction } from 'node:vm'

// Run with the existing staged League source. This records plans without firing any timers.
if (process.argv.length !== 3)
  throw new Error('Usage: node record-auto-select.mjs <staged League source>')
const source = resolve(process.argv[2])
const sourceRequire = createRequire(join(source, 'package.json'))
const ts = sourceRequire('typescript')
const modules = new Map()
const autoSelectDirectory = 'src/main/shards/auto-select'
const fixtureTime = 10_000
let now = fixtureTime
let nextTimer = 0

function loadModule(relativePath) {
  if (modules.has(relativePath)) return modules.get(relativePath)
  const compiled = ts.transpileModule(readFileSync(join(source, relativePath), 'utf8'), {
    fileName: relativePath,
    compilerOptions: {
      target: ts.ScriptTarget.ES2022,
      module: ts.ModuleKind.CommonJS,
      esModuleInterop: true
    }
  })
  const module = { exports: {} }
  const load = (name) => {
    if (name === '@main/i18n') return { i18next: { t: (key) => key } }
    if (name === '@shared/shards/akari-api')
      return loadModule('src/shared/shards/akari-api/auto-select.ts')
    if (name.startsWith('./'))
      return loadModule(posix.join(posix.dirname(relativePath), name + '.ts'))
    return sourceRequire(name)
  }
  compileFunction(compiled.outputText, [
    'exports',
    'require',
    'Date',
    'setTimeout',
    'clearTimeout'
  ])(
    module.exports,
    load,
    { now: () => now },
    () => ++nextTimer,
    () => {}
  )
  modules.set(relativePath, module.exports)
  return module.exports
}

const { AutoSelectState, AutoSelectSettings } = loadModule(autoSelectDirectory + '/state.ts')
const { AutoSelectBenchController } = loadModule(autoSelectDirectory + '/bench-controller.ts')
const { AutoSelectBanPickController } = loadModule(autoSelectDirectory + '/ban-pick-controller.ts')
const { BUILTIN_AUTO_SELECT_GROUPS } = loadModule('src/main/shards/akari-api/builtin.ts')

function sessionAction(action, actorCellId = 1) {
  return {
    ...action,
    type: action.kind,
    championId: action.championId ?? 0,
    actorCellId,
    completed: false
  }
}

function recordPlan(input) {
  now = fixtureTime
  const actions = []
  if (input.activeAction) {
    actions.push([sessionAction(input.activeAction)])
  } else if (input.firstUnfinishedPick) {
    actions.push([sessionAction({ id: 98, kind: 'pick' }, 2)])
  }
  if (input.firstUnfinishedPick && input.activeAction?.kind !== 'pick') {
    actions.push([sessionAction(input.firstUnfinishedPick)])
  }
  const session = {
    actions,
    localPlayerCellId: 1,
    myTeam: [{ cellId: 1, championId: input.currentChampion ?? 0 }],
    benchEnabled: input.benchEnabled,
    benchChampions: (input.benchChampions ?? []).map((championId) => ({ championId })),
    allowDuplicatePicks: input.allowDuplicatePicks,
    allowSubsetChampionPicks: input.allowSubsetPicks,
    timer: {
      phase: input.phase,
      isInfinite: !input.timing,
      adjustedTimeLeftInPhase: input.timing?.remainingMs,
      totalTimeInPhase: input.timing ? input.timing.remainingMs + input.timing.elapsedMs : 0,
      internalNowInEpochMs: fixtureTime
    }
  }
  const gridChampions = Object.fromEntries(
    Object.entries(input.championGrid ?? {}).map(([id, entry]) => [
      id,
      {
        owned: true,
        selectionStatus: {
          isBanned: entry.banned,
          pickIntented: entry.teammateIntent,
          pickedByOtherOrBanned: entry.pickedByOther
        }
      }
    ])
  )
  const leagueClientData = {
    gameflow: {
      session: {
        gameData: {
          isCustomGame: input.isCustomGame ?? false,
          queue: { type: input.queueType, gameMode: input.gameMode }
        }
      }
    },
    champSelect: {
      session,
      gridChampions,
      currentPickableChampionIds: new Set(input.pickable),
      currentBannableChampionIds: new Set(input.bannable)
    },
    lobbyTeamBuilder: { champSelect: { subsetChampionList: input.subsetChampions ?? [] } }
  }
  const configuration = new AutoSelectSettings()
  const prefs = (choice) => ({
    enabled: choice.enabled,
    strategy: choice.strategy,
    champions: { default: choice.champions },
    delaySeconds: choice.delayMs / 1000
  })
  configuration.setPickConfig('aram', {
    ...prefs(input.pick.choice),
    showIntent: input.pick.showIntent ?? false,
    ignoreIntent: input.pick.ignoreIntent ?? false,
    benchSelectFirstAvailableChampion: input.pick.upgradeBench ?? false,
    benchSwapAccumulatedDelaySeconds: input.pick.benchDelayMs / 1000
  })
  configuration.setBanConfig('aram', prefs(input.ban))
  configuration.setPickConfig('ranked', {
    ...prefs(input.pick.choice),
    champions: { top: [11], default: [11] }
  })
  configuration.setBanConfig('ranked', prefs(input.ban))
  const state = new AutoSelectState(
    leagueClientData,
    configuration,
    {
      autoSelectGroups: BUILTIN_AUTO_SELECT_GROUPS
    },
    { availability: { sgpServerId: '' }, leagueServers: { servers: {} } }
  )
  state.setTemporarilyDisabled(input.temporarilyDisabled ?? false)
  const pendingTask = (championId, timerId) => ({
    championId,
    timerId,
    completed: true,
    isPickIntent: false,
    startAt: fixtureTime,
    finishAt: fixtureTime + 50
  })
  if (input.pendingPick != null) state.setDelayedPick(pendingTask(input.pendingPick, -1))
  if (input.pendingBan != null) state.setDelayedBan(pendingTask(input.pendingBan, -2))

  const reactions = []
  const context = {
    state,
    mobxUtils: { reaction: (read, apply) => reactions.push({ read, apply }) },
    logger: { info() {} }
  }
  const localMessages = { send() {} }
  const executor = { championNameWithId: String }
  new AutoSelectBanPickController(context, localMessages, executor).watch()
  new AutoSelectBenchController(context, localMessages, executor).watch()
  const [ban, pick, trackBench, bench] = reactions
  const seen = []
  for (const [id, elapsed] of Object.entries(input.benchElapsedMs ?? {}).sort(
    (a, b) => b[1] - a[1]
  )) {
    now = fixtureTime - elapsed
    seen.push(Number(id))
    trackBench.apply([...seen])
  }
  now = fixtureTime
  ban.apply(ban.read())
  pick.apply(pick.read())
  bench.apply(bench.read())

  const schedule = (task, action) => ({
    decision: 'schedule',
    action,
    championId: task.championId,
    delayMs: task.delayMs
  })
  const { delayedBanTask, delayedPickTask, delayedBenchSwapTask } = state
  if (delayedBanTask?.timerId === -2 || delayedPickTask?.timerId === -1) {
    return { decision: 'keep-scheduled' }
  }
  if (delayedBanTask) {
    return schedule(delayedBanTask, {
      kind: 'ban',
      actionId: state.activeAction.id,
      completed: delayedBanTask.completed
    })
  }
  if (delayedPickTask) {
    return schedule(
      delayedPickTask,
      delayedPickTask.isPickIntent
        ? { kind: 'intent', actionId: state.firstUnfinishedPickAction.id }
        : { kind: 'pick', actionId: state.activeAction.id, completed: delayedPickTask.completed }
    )
  }
  if (delayedBenchSwapTask) return schedule(delayedBenchSwapTask, { kind: 'bench-swap' })
  return { decision: 'idle' }
}

const choice = {
  enabled: true,
  champions: [11, 22, 33],
  strategy: 'show-and-lock-in',
  delayMs: 2000
}
const pick = (choiceOverrides, extras) => ({
  choice: { ...choice, ...choiceOverrides },
  benchDelayMs: 2900,
  ...extras
})
const base = {
  gameMode: 'ARAM',
  queueType: 'ARAM_UNRANKED_5x5',
  phase: 'BAN_PICK',
  pick: pick(),
  ban: choice,
  pickable: [11, 22, 33],
  bannable: [11, 22, 33, -1],
  championGrid: { 11: {}, 22: {}, 33: {} }
}
const choose = (kind, championId = 0) => ({ kind, id: 20, championId })
const bench = {
  benchEnabled: true,
  benchChampions: [22, 11],
  benchElapsedMs: { 22: 2500, 11: 1000 }
}
const shownBan = { activeAction: choose('ban', 33), ban: { ...choice, strategy: 'just-show' } }
const scenarios = [
  [
    'ranked preferences are inert',
    { gameMode: 'CLASSIC', queueType: 'RANKED_SOLO_5x5', activeAction: choose('pick') }
  ],
  ['custom ARAM is inert', { isCustomGame: true, activeAction: choose('pick') }],
  ['missing queue cannot activate', { queueType: null, activeAction: choose('pick') }],
  [
    'temporary pause clears pending',
    { temporarilyDisabled: true, activeAction: choose('pick'), pendingPick: 11 }
  ],
  [
    'missing grid entry and hard pickability',
    { activeAction: choose('pick'), championGrid: { 22: {}, 33: {} }, pickable: [11, 33] }
  ],
  [
    'teammate intent is skipped by default',
    { activeAction: choose('pick'), championGrid: { 11: { teammateIntent: true }, 22: {} } }
  ],
  [
    'intent may be overridden explicitly',
    {
      activeAction: choose('pick'),
      pick: pick({}, { ignoreIntent: true }),
      championGrid: { 11: { teammateIntent: true }, 22: {} }
    }
  ],
  [
    'duplicate mode still rejects banned entries',
    {
      activeAction: choose('pick'),
      allowDuplicatePicks: true,
      championGrid: { 11: { banned: true }, 22: { pickedByOther: true } }
    }
  ],
  [
    'KIWI subset picks lock at first stage',
    {
      gameMode: 'KIWI',
      activeAction: choose('pick'),
      allowSubsetPicks: true,
      subsetChampions: [22, 33],
      timing: { remainingMs: 400, elapsedMs: 700 }
    }
  ],
  [
    'KIWI subset lock retains the first phase offset after showing',
    {
      gameMode: 'KIWI',
      activeAction: choose('pick', 22),
      allowSubsetPicks: true,
      subsetChampions: [22],
      timing: { remainingMs: 4000, elapsedMs: 1750 }
    }
  ],
  [
    'normal second pick stage uses second phase offset',
    {
      activeAction: choose('pick', 11),
      timing: { remainingMs: 27_900, elapsedMs: 2100 }
    }
  ],
  [
    'expired phase budget is zero',
    { activeAction: choose('pick'), timing: { remainingMs: -25, elapsedMs: 30_025 } }
  ],
  [
    'just-show preserves an existing pick',
    { activeAction: choose('pick', 33), pick: pick({ strategy: 'just-show' }) }
  ],
  [
    'planning intent targets first unfinished pick',
    {
      phase: 'PLANNING',
      activeAction: choose('ban'),
      pick: pick({}, { showIntent: true }),
      firstUnfinishedPick: { id: 77, kind: 'pick' }
    }
  ],
  [
    'an existing planning intent is preserved',
    {
      phase: 'PLANNING',
      activeAction: choose('ban'),
      pick: pick({}, { showIntent: true }),
      firstUnfinishedPick: { id: 77, kind: 'pick', championId: 33 }
    }
  ],
  [
    'a future pick without a local active action does not schedule intent',
    {
      phase: 'PLANNING',
      pick: pick({}, { showIntent: true }),
      firstUnfinishedPick: { id: 77, kind: 'pick' }
    }
  ],
  [
    'instant lock keeps the already scheduled same pick',
    {
      activeAction: choose('pick', 11),
      pick: pick({ strategy: 'lock-in-immediately' }),
      pendingPick: 11
    }
  ],
  [
    'ban priority skips teammate intents and already banned',
    {
      activeAction: choose('ban'),
      championGrid: { 11: { teammateIntent: true }, 22: { banned: true }, 33: {} }
    }
  ],
  [
    'show-and-lock-in uses the second phase offset for a ban',
    {
      activeAction: choose('ban', 11),
      timing: { remainingMs: 300, elapsedMs: 3900 }
    }
  ],
  [
    'no-ban sentinel can still be chosen',
    {
      activeAction: choose('ban'),
      ban: { ...choice, champions: [-1] },
      championGrid: { '-1': { banned: true } }
    }
  ],
  ['just-show cancels a pending ban when a hero is already shown', { ...shownBan, pendingBan: 11 }],
  ['bench uses preference priority over bench order', { ...bench, currentChampion: 99 }],
  ['bench holds a preferred hero with upgrade disabled', { ...bench, currentChampion: 33 }],
  [
    'bench never downgrades a preferred hero',
    {
      ...bench,
      currentChampion: 11,
      benchChampions: [22],
      pick: pick({}, { upgradeBench: true })
    }
  ],
  [
    'bench waits for the first priority observation',
    {
      ...bench,
      currentChampion: 99,
      benchElapsedMs: { 22: 2500 }
    }
  ],
  [
    'bench upgrades within the phase budget',
    {
      ...bench,
      currentChampion: 33,
      pick: pick({}, { upgradeBench: true }),
      timing: { remainingMs: 600, elapsedMs: 29_400 }
    }
  ],
  [
    'KIWI bench uses the drawn subset until finalization',
    {
      ...bench,
      currentChampion: 99,
      gameMode: 'KIWI',
      allowSubsetPicks: true,
      subsetChampions: [22]
    }
  ],
  [
    'KIWI bench uses the full bench in finalization',
    {
      ...bench,
      currentChampion: 99,
      gameMode: 'KIWI',
      phase: 'FINALIZATION',
      allowSubsetPicks: true,
      subsetChampions: [22],
      benchElapsedMs: { 11: 4000, 22: 2500 }
    }
  ]
]
const cases = scenarios.map(([name, overrides]) => ({
  name,
  overrides,
  expected: recordPlan({ ...base, ...overrides })
}))
const knownDifference = { overrides: shownBan, original: recordPlan({ ...base, ...shownBan }) }
const rows = cases.map((entry) => '    ' + JSON.stringify(entry)).join(',\n')
console.log(
  '{\n  "base": ' +
    JSON.stringify(base) +
    ',\n  "cases": [\n' +
    rows +
    '\n  ],\n  "justShowWithoutPending": ' +
    JSON.stringify(knownDifference) +
    '\n}'
)
