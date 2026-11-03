// Host-authored fixed-operation controller, not nSkinz implementation.
// CONFIG is serialized by the validated host; marker contents are never commands.
(function () {
  'use strict';
  const C = __COSMETIC_CONFIG__;
  const started = Date.now(), ops = ['position', 'load', 'warm', 'record'];
  let ended=false, seen=false, seq=0, last=0, next=0, active=null;
  let phase='IDLE', phaseStart=started, stable=0, endTick=0;
  let lease=0, leaseTime=started, targetIndex=null;
  function emit(r) { Object.assign(r,{session:C.tag,seq:++seq,script_time_ms:Date.now()}); mirv.message(C.tag+' '+JSON.stringify(r)+'\n'); }
  function path(p) { return typeof p==='string' && /^[a-zA-Z]:[\\/]/.test(p) ? p.replace(/\\/g,'/').toLowerCase() : null; }
  function marker(n) { return new AdvancedfxPath(C.root+'/requests/'+n+'.flag').isFile(); }
  function stop(reason) { if(!ended){ended=true;emit({kind:'STOPPING',reason});mirv.exec('quit');} }
  function entity(h) { if(!mirv.isHandleValid(h)) return null; const e=mirv.getEntityFromIndex(mirv.getHandleEntryIndex(h));return e&&e.isValid()?e:null; }
  function pov() {
    const c=mirv.getEntityFromSplitScreenPlayer(0);
    if(!c || !c.isValid() || !c.isPlayerController()) return null;
    const p=entity(c.getPlayerPawnHandle()); if(!p || !p.isPlayerPawn()) return null;
    const t=entity(p.getObserverTargetHandle()); if(!t || !t.isPlayerPawn()) return null;
    const tc=entity(t.getPlayerControllerHandle()),w=entity(t.getActiveWeaponHandle());
    if(!tc || !tc.isPlayerController() || !w) return null;
    return {xuid:String(tc.getSteamId()),weapon:w.getClassName(),weapon_debug:w.getDebugName(),mode:p.getObserverMode()};
  }
  function sample() {
    if(mirv.isPlayingDemo()!==true) return null;
    const raw=mirv.getDemoFilePath(), resolved=typeof raw==='string'?path(mirv.resolveGamePath(raw)):null;
    const tick=mirv.getDemoTick(),paused=mirv.isDemoPaused(),current=pov();
    if(resolved!==path(C.demo) || mirv.getDemoFilePath()!==raw || mirv.isPlayingDemo()!==true ||
       typeof paused!=='boolean' || !Number.isFinite(tick) || tick<0) throw Error('UNKNOWN_DEMO');
    return {kind:'DEMO_SAMPLE',valid:true,demo:resolved,tick,paused,pov:current,phase,active,visual_verified:null};
  }
  function target(s) {
    const names=C.weaponNames || ['awp','weapon_awp','c_weaponawp'];
    const matches=value => typeof value==='string' && names.includes(value.toLowerCase().split('/').pop());
    return s.pov && s.pov.mode===2 && s.pov.xuid===C.xuid && (matches(s.pov.weapon) || matches(s.pov.weapon_debug));
  }
  function selectTarget() {
    // Parser user IDs are not native controller indices. Resolve the exact
    // Steam ID in this owned Demo session on every seek, never by player name.
    let index=null;
    for(let i=1;i<=64;i++) {
      const controller=mirv.getEntityFromIndex(i);
      if(!controller || !controller.isValid() || !controller.isPlayerController()) continue;
      if(String(controller.getSteamId())!==C.xuid) continue;
      if(index!==null) throw Error('AMBIGUOUS_TARGET_CONTROLLER');
      index=i;
    }
    if(index===null) throw Error('MISSING_TARGET_CONTROLLER');
    targetIndex=index;
    mirv.exec('spec_mode 1');
    mirv.exec('spec_player '+index);
    emit({kind:'TARGET_SELECTED',controller_index:index,xuid:C.xuid});
  }
  function change(p) {phase=p;phaseStart=Date.now();stable=0;}
  function done(s) {emit({kind:'OPERATION_COMPLETE',operation:active,tick:s.tick,pov:s.pov,visual_verified:null});active=null;change('IDLE');}
  emit({kind:'SCRIPT_LOADED'});
  mirv.events.clientFrameStageNotify.on('mgs-cosmetic',function(e){
    if(ended || e.isBefore || e.curStage!==12) return;
    try {
      if(marker('stop')) return stop('REQUESTED_STOP');
      // Lease numbers can only advance; leftover markers cannot renew a lease.
      for(let i=0;i<16 && marker('lease-'+lease);i++){lease++;leaseTime=Date.now();}
      if(Date.now()-leaseTime>8000) return stop('HOST_LEASE_LOST');
      if(Date.now()-started>12*60*1000) return stop('SESSION_DEADLINE');
      const s=sample(); if(!s){if(seen)stop('DEMO_LOST');return;} seen=true;
      if(Date.now()-last>=250){last=Date.now();emit(s);}
      if(phase!=='IDLE' && Date.now()-phaseStart>45000) return stop('OPERATION_TIMEOUT_'+phase);
      if(phase==='SEEKING') {
        // Cold Demo seeks are asynchronous: the initial pause can precede
        // actual seek completion. Pause once the requested region is reached.
        if(!s.paused && Math.abs(s.tick-(C.startTick-164))<=4){mirv.exec('demo_pause');change('SEEK_PAUSING');return;}
        stable=s.paused && Math.abs(s.tick-(C.startTick-164))<=4?stable+1:0;
        if(stable>=4){selectTarget();mirv.exec('demo_resume');endTick=C.startTick-64;change('PREROLL');}
      } else if(phase==='SEEK_PAUSING') {
        if(s.tick>C.startTick-164+64)return stop('SEEK_TICK_OVERRUN');
        stable=s.paused && Math.abs(s.tick-(C.startTick-164))<=4?stable+1:0;
        if(stable>=4){selectTarget();mirv.exec('demo_resume');endTick=C.startTick-64;change('PREROLL');}
      } else if(phase==='PREROLL') {
        if(s.tick>endTick+64) return stop('TICK_OVERRUN');
        if(active==='position') {
          const controller=mirv.getEntityFromIndex(targetIndex);
          if(!controller || !controller.isValid() || !controller.isPlayerController()
             || String(controller.getSteamId())!==C.xuid) return stop('TARGET_CONTROLLER_CHANGED');
          // Recorded spectator commands can override a one-shot command on
          // seek/resume. Hold only during positioning, never hide a target
          // change in an already accepted recording.
          mirv.exec('spec_player '+targetIndex);
        }
        if(active==='record' && !target(s)) return stop('RECORD_TARGET_CHANGED');
        if(s.tick>=endTick){if(!target(s))return stop('TARGET_NOT_READY');mirv.exec('demo_pause');change('PAUSING');}
      } else if(phase==='PAUSING') {
        stable=s.paused && target(s) && Math.abs(s.tick-endTick)<=4?stable+1:0;
        if(stable>=4)done(s);
      } else if(phase==='IDLE' && next<ops.length && marker(ops[next])) {
        active=ops[next++];emit({kind:'COMMAND_SUBMITTED',operation:active});
        if(active==='position'){mirv.exec('demo_gototick '+(C.startTick-164));mirv.exec('demo_pause');change('SEEKING');}
        else if(active==='load'){
          if(!s.paused || !target(s) || Math.abs(s.tick-(C.startTick-64))>4)return stop('LOAD_TARGET_MISMATCH');
          if(C.extensionMode==='file') {
            if(C.skyCommands!==undefined){
              const safe=command=>/^mirv_sky material (default|"materials\/skybox\/[a-zA-Z0-9_]+\.vmat_c")$/.test(command)
                || ['mirv_sky color default','mirv_sky clouds color default','mirv_sky clouds draw 1'].includes(command);
              if(!Array.isArray(C.skyCommands)||!C.skyCommands.length||!C.skyCommands.every(safe))return stop('UNKNOWN_SKY_COMMAND');
              C.skyCommands.forEach(command=>mirv.exec(command));
            }
            // The host independently verified a dedicated playback copy. No
            // runtime cosmetic DLL is loaded in this provider's session.
            // Clear the game HUD before the warm/preview phase; record must
            // never be the first point at which presentation is changed.
            mirv.exec('sv_cheats 1');
            mirv.exec('demoui false');
            mirv.exec('cl_draw_only_deathnotices true');
            mirv.exec('spec_show_xray 0');
            // OBS/UI can take focus. Keep this isolated Demo's game audio
            // rendering while the process-scoped capture records it.
            mirv.exec('snd_mute_losefocus false');
          } else if(C.extensionMode===undefined || C.extensionMode==='nskinz') {
            mirv.exec('mirv_loadlibrary "'+C.dll+'"');
          } else return stop('UNKNOWN_EXTENSION_MODE');
          done(s);
        } else {
          if(!s.paused || !target(s))return stop('TARGET_MISMATCH');
          endTick=active==='warm'?C.startTick:C.endTick;mirv.exec('demo_resume');change('PREROLL');
        }
      }
    } catch(_){stop('API_OR_STATE_EXCEPTION');}
  });
})();
