// Node VM tests for the host-authored runtime only. No game, DLL, network,
// subprocess, or native loader is used.
const assert = require('assert');
const fs = require('fs');
const vm = require('vm');

const RUNTIME = require('path').resolve(__dirname, '../app/cosmetic_studio/runtime.js');
const DEMO = 'E:/Demos/fixture.dem';
const CONFIG = {
  tag: 'MGS_TEST_RUNTIME', root: 'C:/MGS/session', demo: DEMO,
  xuid: '76561199059074958', slot: 4, startTick: 10000, endTick: 10128,
  dll: 'C:/Pinned/nSkinz.dll',
};

function harness(config = CONFIG, overrides = {}) {
  let now = 1_000_000;
  const markers = new Set(overrides.markers || []);
  const records = [], commands = [];
  let callback;
  const state = {
    playing: true, path: DEMO, paused: true, tick: config.startTick - 64,
    xuid: config.xuid, weapon: 'weapon_awp', mode: 2, ...overrides.state,
  };
  const mirv = {
    isPlayingDemo: () => state.playing,
    getDemoFilePath: () => state.path,
    resolveGamePath: p => p,
    isDemoPaused: () => state.paused,
    getDemoTick: () => state.tick,
    isHandleValid: () => true,
    getEntityFromSplitScreenPlayer: () => ({ isValid: () => true, isPlayerController: () => true, getPlayerPawnHandle: () => 'pawn' }),
    getEntityFromIndex: handle => {
      if (!overrides.noController && (handle === (overrides.controllerIndex || config.slot)
          || handle === overrides.secondControllerIndex)) return {isValid: () => true,
        isPlayerController: () => true, getSteamId: () => config.xuid};
      if (handle === 'pawn') return { isValid: () => true, isPlayerPawn: () => true, getObserverTargetHandle: () => 'target', getObserverMode: () => state.mode };
      if (handle === 'target') return { isValid: () => true, isPlayerPawn: () => true, getPlayerControllerHandle: () => 'controller', getActiveWeaponHandle: () => 'weapon' };
      if (handle === 'controller') return { isValid: () => true, isPlayerController: () => true, getSteamId: () => state.xuid };
      if (handle === 'weapon') return { isValid: () => true, getClassName: () => state.weapon, getDebugName: () => state.weapon };
      return null;
    },
    getHandleEntryIndex: h => h,
    message: line => records.push(JSON.parse(line.slice(line.indexOf(' ') + 1))),
    exec: command => {
      commands.push(command);
      if (command === 'demo_pause') state.paused = true;
      if (command === 'demo_resume') state.paused = false;
    },
    events: { clientFrameStageNotify: { on: (_id, fn) => { callback = fn; } } },
  };
  const context = {
    mirv,
    AdvancedfxPath: class { constructor(path) { this.path = path; } isFile() { return markers.has(this.path.split('/').pop().replace('.flag', '')); } },
    Date: { now: () => now }, Number, Error, JSON, Math, String,
  };
  const source = fs.readFileSync(RUNTIME, 'utf8').replace('__COSMETIC_CONFIG__', JSON.stringify(config));
  vm.runInNewContext(source, context, { filename: RUNTIME });
  function frame() { callback({ isBefore: false, curStage: 12 }); }
  return { state, records, commands, markers, frame, advance: ms => { now += ms; } };
}

function stopReason(h) { return h.records.find(r => r.kind === 'STOPPING')?.reason; }
function completed(h, op) { return h.records.filter(r => r.kind === 'OPERATION_COMPLETE' && r.operation === op); }
function submitted(h, op) { return h.records.filter(r => r.kind === 'COMMAND_SUBMITTED' && r.operation === op); }

function completePosition(h) {
  h.markers.add('position'); h.frame();
  h.state.tick = CONFIG.startTick - 164;
  for (let i = 0; i < 4; i++) h.frame();
  h.state.tick = CONFIG.startTick - 64; h.frame();
  for (let i = 0; i < 4; i++) h.frame();
  assert.strictEqual(completed(h, 'position').length, 1);
}

const liveIndex = harness(CONFIG, {controllerIndex:13});
completePosition(liveIndex);
assert(liveIndex.commands.includes('spec_player 13'), 'native XUID/controller index must replace parser slot hint');
assert(!liveIndex.commands.includes('spec_player 4'));
assert(liveIndex.commands.filter(c=>c==='spec_player 13').length>=2,
  'positioning must hold the controller through asynchronous seek/resume');

for (const [options,reason] of [[{noController:true},'MISSING_TARGET_CONTROLLER'],
    [{controllerIndex:13,secondControllerIndex:14},'AMBIGUOUS_TARGET_CONTROLLER']]) {
  const invalid=harness(CONFIG,options);
  invalid.markers.add('position');invalid.frame();
  invalid.state.tick=CONFIG.startTick-164;
  for(let i=0;i<5;i++)invalid.frame();
  assert.strictEqual(stopReason(invalid),'API_OR_STATE_EXCEPTION',reason);
  assert(!invalid.commands.some(command=>command.startsWith('spec_player ')),
    'Unknown or duplicate XUID must never fall back to the parser slot');
}

function completeSimple(h, operation, endTick) {
  h.markers.add(operation); h.frame();
  h.state.tick = endTick; h.frame();
  for (let i = 0; i < 4; i++) h.frame();
  assert.strictEqual(completed(h, operation).length, 1);
}

function testExactDemoAndLostDemo() {
  const wrong = harness(CONFIG, { state: { path: 'E:/Demos/other.dem' } });
  wrong.frame();
  assert.strictEqual(wrong.commands.filter(c => c === 'quit').length, 1);
  assert.strictEqual(stopReason(wrong), 'API_OR_STATE_EXCEPTION');
  const lost = harness(); lost.frame(); lost.state.playing = false; lost.frame(); lost.frame();
  assert.strictEqual(stopReason(lost), 'DEMO_LOST');
  assert.strictEqual(lost.commands.filter(c => c === 'quit').length, 1);
}

function testLeaseMonotonicAndStaleRenewalDoesNotKeepAlive() {
  const h = harness(); h.markers.add('lease-0'); h.frame();
  h.markers.delete('lease-0');
  h.advance(8001); h.frame();
  assert.strictEqual(stopReason(h), 'HOST_LEASE_LOST');
}

function testOperationOrderSeekSettlingAndLoadGate() {
  const h = harness(); completePosition(h);
  h.markers.add('load'); h.frame();
  assert.strictEqual(completed(h, 'load').length, 1);
  assert.deepStrictEqual(submitted(h, 'position').map(r => r.operation), ['position']);
  assert(h.commands.includes('demo_gototick 9836'));
  assert(h.commands.includes('mirv_loadlibrary "C:/Pinned/nSkinz.dll"'));
}

function testPausedTargetMismatchDoesNotLoad() {
  const h = harness(); completePosition(h);
  h.markers.add('load'); h.state.xuid = '76561199059074959'; h.frame();
  assert.strictEqual(stopReason(h), 'LOAD_TARGET_MISMATCH');
  assert(!h.commands.some(c => c.startsWith('mirv_loadlibrary')));
}

function testExactXuidStringAndRecordingTargetChangeStop() {
  const h = harness(); completePosition(h); h.markers.add('load'); h.frame();
  completeSimple(h, 'warm', CONFIG.startTick);
  h.markers.add('record'); h.frame();
  h.state.xuid = '76561199059074959'; h.state.tick = CONFIG.endTick; h.frame();
  assert.strictEqual(stopReason(h), 'RECORD_TARGET_CHANGED');
}

function testStopIsIdempotent() {
  const h = harness(); h.markers.add('stop'); h.frame(); h.frame(); h.frame();
  assert.strictEqual(stopReason(h), 'REQUESTED_STOP');
  assert.strictEqual(h.records.filter(r => r.kind === 'STOPPING').length, 1);
  assert.strictEqual(h.commands.filter(c => c === 'quit').length, 1);
}

function testNonFirstPersonDoesNotLoad() {
  const h = harness(); completePosition(h);
  h.state.mode = 4; h.markers.add('load'); h.frame();
  assert.strictEqual(stopReason(h), 'LOAD_TARGET_MISMATCH');
  assert(!h.commands.some(c => c.startsWith('mirv_loadlibrary')));
}

function testColdSeekPauseAfterArrival() {
  const h = harness(); h.markers.add('position'); h.frame();
  h.state.paused = false; h.state.tick = CONFIG.startTick - 164 + 1;
  h.frame();
  assert.strictEqual(h.commands.filter(c => c === 'demo_pause').length, 2);
  for(let i=0;i<4;i++)h.frame();
  assert(h.commands.includes('spec_player 4'));
  h.state.tick=CONFIG.startTick-64;h.frame();
  for(let i=0;i<4;i++)h.frame();
  assert.strictEqual(completed(h,'position').length,1);
  assert(!h.commands.some(c=>c.startsWith('mirv_loadlibrary')));
}

const tests = [
  ['exact_demo_and_lost_demo', testExactDemoAndLostDemo],
  ['lease_monotonic_stale_renewal', testLeaseMonotonicAndStaleRenewalDoesNotKeepAlive],
  ['operation_order_seek_settling_load_gate', testOperationOrderSeekSettlingAndLoadGate],
  ['paused_target_mismatch_no_load', testPausedTargetMismatchDoesNotLoad],
  ['exact_xuid_recording_target_change', testExactXuidStringAndRecordingTargetChangeStop],
  ['stop_idempotent', testStopIsIdempotent],
  ['cold_seek_pause_after_arrival', testColdSeekPauseAfterArrival],
  ['non_first_person_does_not_load', testNonFirstPersonDoesNotLoad],
];
const results = [];
for (const [name, fn] of tests) {
  try { fn(); results.push({ name, passed: true }); }
  catch (error) { results.push({ name, passed: false, error: error.message }); }
}
const failed = results.filter(r => !r.passed);
console.log(JSON.stringify({ scope: 'NODE_MOCK_RUNTIME_ONLY', tests: results, passed: failed.length === 0 }, null, 2));
process.exitCode = failed.length ? 1 : 0;
