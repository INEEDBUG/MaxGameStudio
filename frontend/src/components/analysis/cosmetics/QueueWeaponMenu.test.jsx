import {fireEvent, render, screen} from '@testing-library/react';
import {expect, test, vi} from 'vitest';
import QueueWeaponMenu from './QueueWeaponMenu.jsx';

const props={draft:{},presets:new Map(),renderArtwork:()=>null,reason:item=>item.editable?'':item.reason||'未读取到装备'};
test('catalogue is complete even when the Demo only contains one gun',()=>{
 render(<QueueWeaponMenu {...props} items={[{slot_id:'a',definition:9,name:'AWP | 猫猫狗狗',team:'both',editable:true}]} onSelect={vi.fn()}/>);
 expect(document.querySelectorAll('[data-weapon-definition]')).toHaveLength(34);
 expect(screen.getByRole('button',{name:/FAMAS/})).toBeTruthy();
 expect(screen.getByRole('button',{name:/加利尔/})).toBeTruthy();
 fireEvent.click(screen.getByRole('button',{name:'T',exact:true}));
 expect(document.querySelectorAll('[data-weapon-definition]')).toHaveLength(34);
});
test('foreign and repeated same-model records cannot create duplicate weapon buttons or shadow an owned gun',()=>{
 const own={slot_id:'own',definition:9,name:'AWP | 自己的枪',team:'ct',editable:true};
 const picked={slot_id:'foreign',definition:9,name:'AWP | 别人的枪',team:'ct',editable:false,reason:'归属不同'};
 const select=vi.fn();render(<QueueWeaponMenu {...props} items={[picked,own,{...picked,slot_id:'foreign-two'}]} onSelect={select}/>);
 expect(document.querySelectorAll('[data-weapon-definition="9"]')).toHaveLength(1);
 fireEvent.click(screen.getByRole('button',{name:'更换 AWP | 自己的枪'}));
 expect(select.mock.calls.at(-1)[0].slot_id).toBe('own');
});
test('unavailable guns are inspectable but remain unavailable to the skin editor',()=>{
 const select=vi.fn();render(<QueueWeaponMenu {...props} items={[]} onSelect={select}/>);
 fireEvent.click(screen.getByRole('button',{name:/FAMAS/}));
 expect(select.mock.calls.at(-1)[0]).toMatchObject({definition:10,editable:false,preset_ids:[],reason:'not_observed'});
});
