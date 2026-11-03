"""Expand a player's weapon choice to verified whole-Demo identities, not ticks."""
from .appearance_models import AppearanceReplacement


def expand_match_plan(items, xuid, choices):
    slots = {item.slot_id:item for item in items if item.xuid==xuid}
    rules = {}
    for choice in choices:
        item = slots.get(choice.slot_id)
        if item is None or not item.editable:
            raise ValueError('整局方案的原始装备已变化，请重新读取。')
        definition = item.definition
        value = (choice.preset_id, choice.wear, choice.seed)
        if definition in rules and rules[definition] != value:
            raise ValueError('同一武器选择了不同皮肤；整局模式请统一选择。')
        rules[definition] = value
    expanded, skipped = [], 0
    for item in slots.values():
        if item.definition not in rules:
            continue
        preset, wear, seed = rules[item.definition]
        if not item.editable:
            if item.reason == 'foreign_pickup':
                skipped += 1  # Explicitly identified foreign pickups only.
                continue
            raise ValueError('整局中存在未适配或未确认的同型号装备，未生成部分替换结果。请使用高级逐件编辑。')
        if preset not in item.preset_ids:
            raise ValueError('整局中存在不能应用此皮肤的装备，未生成部分替换结果。')
        expanded.append(AppearanceReplacement(slot_id=item.slot_id,preset_id=preset,wear=wear,seed=seed))
    if len(expanded)>128:
        raise ValueError('整局装备数量超出本地验证上限。')
    return expanded, skipped
