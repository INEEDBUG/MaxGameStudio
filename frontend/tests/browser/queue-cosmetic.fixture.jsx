import React,{useState} from 'react';
import {createRoot} from 'react-dom/client';
import QueueInspectorPanel from '../../src/components/recordingQueue/QueueInspectorPanel';
import {useRecordingQueue} from '../../src/stores/recordingQueueStore';
import '../../src/index.css';
import '../../src/styles/liquidGlass.css';
const item={id:'one',demoPath:'D:/fixture.dem',demoFilename:'fixture.dem',targetPlayer:'Alice',targetSteamId:'76561199369875913',
 clipData:{round:7,map_name:'de_dust2',category:'highlight',start_tick:1000,end_tick:2000,victims:[]}};
useRecordingQueue.setState({queue:[item,{...item,id:'two'},{...item,id:'other',targetPlayer:'Bob',targetSteamId:'76561199369875914'}]});
window.queueTest={get:()=>useRecordingQueue.getState().queue};
function Fixture(){const queue=useRecordingQueue(s=>s.queue);const [id,setId]=useState('one');return <div className="h-screen flex flex-col overflow-hidden bg-cs2-bg-page text-cs2-text-primary">
 <header className="h-11 shrink-0 px-6 py-2">MaxGameStudio · 录制控制中心</header><div className="flex flex-1 min-h-0">
 <main className="flex-1 p-5">{queue.map(row=><button key={row.id} className="block p-4" onClick={()=>setId(row.id)}>片段 {row.id} · {row.targetPlayer}</button>)}</main>
 <aside className="w-[340px] min-h-0"><QueueInspectorPanel selectedId={id} selectedItem={queue.find(row=>row.id===id)} queue={queue}/></aside>
 </div></div>}
createRoot(document.getElementById('root')).render(<Fixture/>);
