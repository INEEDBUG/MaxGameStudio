import {beforeEach,expect,test} from 'vitest';
import {useRecordingQueue} from './recordingQueueStore.js';
beforeEach(()=>useRecordingQueue.setState({queue:[
 {id:'one',demoPath:'D:/source.dem',targetSteamId:'123'},
 {id:'two',demoPath:'D:/source.dem',targetSteamId:'123'},
 {id:'other-player',demoPath:'D:/source.dem',targetSteamId:'456'},
 {id:'other-demo',demoPath:'D:/other.dem',targetSteamId:'123'}]}));
test('shared appearance changes only explicitly matching Demo and target',()=>{
 const profile={expected_demo_sha256:'a'.repeat(64),replacements:[{slot_id:'owned:usp',preset_id:'usp-kill-confirmed',wear:.01,seed:0}],scope:'match'};
 useRecordingQueue.getState().updateItemAppearance('one',profile,true);
 const rows=useRecordingQueue.getState().queue;
 expect(rows[0].appearance).toEqual(profile);expect(rows[1].appearance).toEqual(profile);
 expect(rows[2].appearance).toBeUndefined();expect(rows[3].appearance).toBeUndefined();
 profile.replacements[0].seed=55;
 expect(rows[0].appearance.replacements[0].seed).toBe(0);
});
test('restoring a single clip does not reset its sibling',()=>{
 useRecordingQueue.getState().updateItemAppearance('one',{replacements:[{slot_id:'a'}]},true);
 useRecordingQueue.getState().updateItemAppearance('one',null,false);
 const rows=useRecordingQueue.getState().queue;
 expect(rows[0].appearance).toBeNull();expect(rows[1].appearance).not.toBeNull();
});
