"""Curated local-only VALORANT display presets.

The entries are independently compiled reference workflows.  They are not a
copy of a third-party player database and are not endorsed by Riot Games.
"""

from __future__ import annotations

from copy import deepcopy


_COMMON_WORKFLOW = {
    "aspect_method": "fill",
    "refresh_policy": "preserve_current",
    "gpu_scaling": "full_panel",
    "monitor_policy": "detect_and_skip",
    "game_steps": [
        {
            "display_mode": "windowed_fullscreen",
            "aspect_method": "fill",
            "apply": True,
        },
        {
            "display_mode": "fullscreen",
            "aspect_method": "fill",
            "apply": True,
        },
    ],
}


DISPLAY_PRESETS = [
    {
        "id": "community-1568x1080",
        "name": "1568 × 1080",
        "width": 1568,
        "height": 1080,
        "derived_aspect_ratio": "1.452:1",
        "aspect_label": "社区 1.45:1",
        "badge": "社区热门",
        "category": "experimental_community",
        "description": "国内外真实拉伸社区常见方案；并非标准 4:3。",
        "recommended": True,
        "confidence": "community_observed",
        **deepcopy(_COMMON_WORKFLOW),
    },
    {
        "id": "classic-1440x1080",
        "name": "1440 × 1080",
        "width": 1440,
        "height": 1080,
        "derived_aspect_ratio": "1.333:1",
        "aspect_label": "4:3",
        "badge": "清晰 4:3",
        "category": "standard_aspect",
        "description": "在 1080p 面板上保持较高纵向清晰度。",
        "recommended": False,
        "confidence": "common_reference",
        **deepcopy(_COMMON_WORKFLOW),
    },
    {
        "id": "classic-1280x960",
        "name": "1280 × 960",
        "width": 1280,
        "height": 960,
        "derived_aspect_ratio": "1.333:1",
        "aspect_label": "4:3",
        "badge": "经典 4:3",
        "category": "standard_aspect",
        "description": "职业设置资料中较常见的 4:3 分辨率。",
        "recommended": False,
        "confidence": "common_reference",
        **deepcopy(_COMMON_WORKFLOW),
    },
    {
        "id": "performance-1024x768",
        "name": "1024 × 768",
        "width": 1024,
        "height": 768,
        "derived_aspect_ratio": "1.333:1",
        "aspect_label": "4:3",
        "badge": "低负载",
        "category": "standard_aspect",
        "description": "更偏向性能，但图像清晰度明显降低。",
        "recommended": False,
        "confidence": "common_reference",
        **deepcopy(_COMMON_WORKFLOW),
    },
    {
        "id": "balanced-1680x1050",
        "name": "1680 × 1050",
        "width": 1680,
        "height": 1050,
        "derived_aspect_ratio": "1.600:1",
        "aspect_label": "16:10",
        "badge": "轻度拉伸",
        "category": "standard_aspect",
        "description": "变形幅度较轻，适合作为过渡方案。",
        "recommended": False,
        "confidence": "common_reference",
        **deepcopy(_COMMON_WORKFLOW),
    },
    {
        "id": "native-1920x1080",
        "name": "1920 × 1080",
        "width": 1920,
        "height": 1080,
        "derived_aspect_ratio": "1.778:1",
        "aspect_label": "16:9",
        "badge": "原生参考",
        "category": "native_reference",
        "description": "用于对照与一键恢复，不属于拉伸方案。",
        "recommended": False,
        "confidence": "standard",
        **deepcopy(_COMMON_WORKFLOW),
    },
]


PRESET_SOURCE_NOTE = {
    "retrieved_at": "2026-08-26",
    "label": "独立编制的社区与职业设置参考",
    "disclaimer": (
        "预设不是 Riot 或 ProSettings 官方配置；公开资料会变化，"
        "实际可用性取决于显卡驱动、显示器和当前游戏版本。"
    ),
    "references": [
        "https://prosettings.net/lists/valorant/",
        "https://prosettings.net/guides/valorant-options/",
        "https://www.reddit.com/r/ValorantTechSupport/comments/1uj49nr/",
    ],
}


def list_display_presets() -> dict:
    return {
        "items": deepcopy(DISPLAY_PRESETS),
        "source": deepcopy(PRESET_SOURCE_NOTE),
    }

