// Isolated VM checks for provider extension-mode policy. This never starts a
// process and never loads a DLL; it only executes the host-authored controller.
const assert = require('assert');
const fs = require('fs');
const path = require('path');
const vm = require('vm');

const runtimePath = path.resolve(__dirname, '../app/cosmetic_studio/runtime.js');
const base = {
  tag: 'MGS_FILE_MODE_TEST', root: 'C:/MGS/session', demo: 'E:/Demos/fixture.dem',
  xuid: '76561199059074958', slot: 4, startTick: 10000, endTick: 10128,
  dll: 'C:/Pinned/nSkinz.dll', extensionMode: 'file',
};

function harness(extensionMode, beforePosition = () => {}) {
  const state = { playing: true, paused: true, tick: 9936, xuid: base.xuid, mode: 2 };
  const markers = new Set(['load']);
  const records = [], commands = [];
  let frame;
  const mirv = {
    isPlayingDemo: () => state.playing,
    getDemoFilePath: () => base.demo,
    resolveGamePath: value => value,
    isDemoPaused: () => state.paused,
    getDemoTick: () => state.tick,
    isHandleValid: () => true,
    getHandleEntryIndex: value => value,
    getEntityFromSplitScreenPlayer: () => ({
      isValid: () => true, isPlayerController: () => true, getPlayerPawnHandle: () => 'pawn',
    }),
    getEntityFromIndex: handle => {
      if (handle === base.slot) return {isValid: () => true, isPlayerController: () => true,
        getSteamId: () => base.xuid};
      if (handle === 'pawn') return { isValid: () => true, isPlayerPawn: () => true,
        getObserverTargetHandle: () => 'target', getObserverMode: () => state.mode };
      if (handle === 'target') return { isValid: () => true, isPlayerPawn: () => true,
        getPlayerControllerHandle: () => 'controller', getActiveWeaponHandle: () => 'weapon' };
      if (handle === 'controller') return { isValid: () => true, isPlayerController: () => true,
        getSteamId: () => state.xuid };
      if (handle === 'weapon') return { isValid: () => true, getClassName: () => 'weapon_awp',
        getDebugName: () => 'weapon_awp' };
      return null;
    },
    message: line => records.push(JSON.parse(line.slice(line.indexOf(' ') + 1))),
    exec: command => { commands.push(command); if (command === 'demo_pause') state.paused = true; if (command === 'demo_resume') state.paused = false; },
    events: { clientFrameStageNotify: { on: (_id, callback) => { frame = callback; } } },
  };
  const context = {
    mirv,
    AdvancedfxPath: class { constructor(value) { this.value = value; }
      isFile() { return markers.has(this.value.split('/').pop().replace('.flag', '')); } },
    Date: { now: () => 1_000_000 }, Number, Error, JSON, Math, String,
  };
  const config = { ...base, extensionMode };
  vm.runInNewContext(fs.readFileSync(runtimePath, 'utf8')
    .replace('__COSMETIC_CONFIG__', JSON.stringify(config)), context, { filename: runtimePath });
  markers.add('position');
  frame({ isBefore: false, curStage: 12 });
  state.tick = 9836;
  for (let i = 0; i < 4; i += 1) frame({ isBefore: false, curStage: 12 });
  state.tick = 9936;
  frame({ isBefore: false, curStage: 12 });
  for (let i = 0; i < 4; i += 1) frame({ isBefore: false, curStage: 12 });
  beforePosition(state);
  frame({ isBefore: false, curStage: 12 });
  return { state, markers, records, commands, frame: () => frame({ isBefore: false, curStage: 12 }) };
}

const fileMode = harness('file');
assert(fileMode.records.some(record => record.kind === 'OPERATION_COMPLETE' && record.operation === 'load'));
assert(!fileMode.commands.some(command => command.startsWith('mirv_loadlibrary')));
assert(fileMode.commands.includes('sv_cheats 1'));
assert(fileMode.commands.includes('demoui false'));
assert(fileMode.commands.includes('cl_draw_only_deathnotices true'));
assert(fileMode.commands.includes('snd_mute_losefocus false'));
assert(fileMode.commands.includes('spec_show_xray 0'));

function completeWarm(h) {
  h.markers.add('warm');
  h.frame();
  h.state.tick = 10000;
  h.frame();
  for (let i = 0; i < 4; i += 1) h.frame();
}

const hudMode = harness('file');
completeWarm(hudMode);
hudMode.markers.add('record');
hudMode.frame();
assert.strictEqual(hudMode.commands.filter(command => command === 'sv_cheats 1').length, 1);
assert.strictEqual(hudMode.commands.filter(command => command === 'demoui false').length, 1);
assert.strictEqual(hudMode.commands.filter(command => command === 'cl_draw_only_deathnotices true').length, 1);

const unpaused = harness('file', state => { state.paused = false; });
assert.strictEqual(unpaused.records.find(record => record.kind === 'STOPPING')?.reason, 'LOAD_TARGET_MISMATCH');
assert(!unpaused.commands.includes('cl_draw_only_deathnotices true'));
assert(!unpaused.commands.includes('snd_mute_losefocus false'));

const wrongTarget = harness('file', state => { state.xuid = '76561199059074959'; });
assert.strictEqual(wrongTarget.records.find(record => record.kind === 'STOPPING')?.reason, 'LOAD_TARGET_MISMATCH');
assert(!wrongTarget.commands.includes('cl_draw_only_deathnotices true'));
assert(!wrongTarget.commands.includes('snd_mute_losefocus false'));

const unknownMode = harness('unrecognized');
assert.strictEqual(unknownMode.records.find(record => record.kind === 'STOPPING')?.reason,
  'UNKNOWN_EXTENSION_MODE');
assert(!unknownMode.commands.some(command => command.startsWith('mirv_loadlibrary')));

const nskinzMode = harness('nskinz');
assert(nskinzMode.commands.some(command => command.startsWith('mirv_loadlibrary')));
assert(!nskinzMode.commands.includes('sv_cheats 1'));
assert(!nskinzMode.commands.includes('demoui false'));
assert(!nskinzMode.commands.includes('cl_draw_only_deathnotices true'));
assert(!nskinzMode.commands.includes('snd_mute_losefocus false'));

console.log(JSON.stringify({ scope: 'NODE_MOCK_RUNTIME_EXTENSION_MODE', tests: 6, passed: true }));
