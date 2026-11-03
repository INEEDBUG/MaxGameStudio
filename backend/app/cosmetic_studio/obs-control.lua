-- Independent portable OBS controller; fixed files, no socket or game hook.
local obs=obslua
local root=__COSMETIC_ROOT__
local requested=false
local stopped=false
local started_at=nil
local stop_reason=nil
local last_lease=nil
local function exists(name)
  local f=io.open(root..'/requests/'..name,'r')
  if not f then return false end
  f:close();return true
end
local function alive(now)
  local f=io.open(root..'/requests/obs-lease.txt','r')
  if f then
    local stamp=tonumber(f:read('*a'));f:close()
    if stamp~=nil and stamp==math.floor(stamp) and now-stamp>=0 and now-stamp<=3
       and (last_lease==nil or stamp>=last_lease) then last_lease=stamp end
  end
  -- A Windows reader may briefly race atomic replacement. A read failure
  -- never renews the last timestamp; its original three-second lease expires.
  return last_lease~=nil and now-last_lease>=0 and now-last_lease<=3
end
local function status(reason)
  local width,height=0,0
  local source=obs.obs_get_source_by_name('Offline CS2 Demo')
  if source then
    width=obs.obs_source_get_width(source)
    height=obs.obs_source_get_height(source)
    obs.obs_source_release(source)
  end
  local f=io.open(root..'/evidence/obs-status.jsonl','a')
  if f then
    f:write(string.format('{"time":%d,"active":%s,"requested":%s,"stopped":%s,"reason":"%s","width":%d,"height":%d}\n',os.time(),tostring(obs.obs_frontend_recording_active()),tostring(requested),tostring(stopped),stop_reason or reason,width,height))
    f:close()
  end
end
local function poll()
  local now=os.time()
  local host_alive=alive(now)
  if exists('obs-stop.flag') and not stopped then
    stopped=true
    stop_reason='REQUESTED_STOP'
    if obs.obs_frontend_recording_active() then obs.obs_frontend_recording_stop() end
  end
  if requested and not stopped and (not host_alive or now<started_at or now-started_at>40) then
    stop_reason=not host_alive and 'HOST_LEASE_LOST' or now<started_at and 'CLOCK_CHANGED' or 'RECORDING_DEADLINE'
    if not stopped then obs.obs_frontend_recording_stop();stopped=true end
  end
  if not requested and not stopped and exists('obs-start.flag') and host_alive then
    requested=true;started_at=now;obs.obs_frontend_recording_start()
  end
  status('TIMER')
end
function script_description() return 'MaxGameStudio isolated offline recording controller.' end
function script_load(settings) obs.timer_add(poll,100);status('LOADED') end
function script_unload()
  obs.timer_remove(poll)
  if requested and obs.obs_frontend_recording_active() then obs.obs_frontend_recording_stop() end
end
