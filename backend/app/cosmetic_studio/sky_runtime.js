// Whole-Demo presentation session. No cosmetic DLL and no arbitrary command input.
(function () {
  const C=__SKY_CONFIG__;
  let ended=false,seen=false,lease=0,leaseTime=Date.now(),last=0,lastTick=-1,applied=false,pauseRequested=false;
  const started=Date.now();
  const path=p=>String(p).replace(/\\/g,'/').toLowerCase();
  const marker=name=>new AdvancedfxPath(C.root+'/requests/'+name+'.flag').isFile();
  const emit=row=>mirv.message(C.tag+' '+JSON.stringify({...row,session:C.tag,script_time_ms:Date.now(),visual_verified:null}));
  const safe=command=>/^mirv_sky material (default|"materials\/skybox\/[a-zA-Z0-9_]+\.vmat_c")$/.test(command)
    || ['mirv_sky color default','mirv_sky clouds color default','mirv_sky clouds draw 1'].includes(command);
  function stop(reason,knownDemo=false) {
    if(ended)return;ended=true;
    emit({kind:'STOPPING',reason});
    if(knownDemo)mirv.exec('mirv_sky material default');
    mirv.exec('quit');
  }
  function apply() {
    if(!Array.isArray(C.commands)||!C.commands.length||!C.commands.every(safe))throw Error('BAD_SKY');
    C.commands.forEach(command=>mirv.exec(command));
    applied=true;emit({kind:'SKY_COMMANDS_SUBMITTED'});
  }
  emit({kind:'SCRIPT_LOADED'});
  mirv.events.clientFrameStageNotify.on('mgs-sky',function(e){
    if(ended||e.isBefore||e.curStage!==12)return;
    try {
      for(let i=0;i<16&&marker('lease-'+lease);i++){lease++;leaseTime=Date.now();}
      if(Date.now()-leaseTime>8000)return stop('HOST_LEASE_LOST');
      if(Date.now()-started>45*60*1000)return stop('SESSION_DEADLINE');
      if(mirv.isPlayingDemo()!==true){if(seen)stop('DEMO_LOST');return;}
      const raw=mirv.getDemoFilePath();
      const resolved=typeof raw==='string'?path(mirv.resolveGamePath(raw)):null;
      const tick=mirv.getDemoTick(),paused=mirv.isDemoPaused();
      if(resolved!==path(C.demo)||mirv.getDemoFilePath()!==raw||mirv.isPlayingDemo()!==true||typeof paused!=='boolean'||!Number.isFinite(tick)||tick<0)
        return stop('UNKNOWN_DEMO');
      seen=true;
      if(marker('stop'))return stop('REQUESTED_STOP',true);
      if(marker('pause')&&!pauseRequested){pauseRequested=true;mirv.exec('demo_pause');}
      // First application waits for normal native initialization; seek/reload
      // backwards re-applies the material rather than leaving the default sky.
      if((!applied&&tick>=1024)||(applied&&tick<lastTick))apply();
      lastTick=tick;
      if(Date.now()-last>=250){last=Date.now();emit({kind:'DEMO_SAMPLE',valid:true,demo:resolved,tick,paused,
        phase:'IDLE',pov:null,sky_submitted:applied});}
    } catch(_){stop('API_OR_STATE_EXCEPTION');}
  });
})();
