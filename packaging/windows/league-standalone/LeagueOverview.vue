<template>
  <NScrollbar>
    <main class="league-overview">
      <h1>{{ english ? 'Overview' : '总览' }}</h1>
      <section class="client-panel" aria-live="polite">
        <span class="league-game-mark" aria-hidden="true"></span>
        <div>
          <strong>{{ clientStatus }}</strong>
          <p>{{ phase }}</p>
        </div>
      </section>

      <div class="overview-columns">
        <section class="overview-panel">
          <h2>{{ english ? 'Quick settings' : '常用功能' }}</h2>
          <div class="settings-row" v-for="setting in quickSettings" :key="setting.id">
            <div><strong>{{ setting.label }}</strong><p>{{ setting.description }}</p></div>
            <NSwitch :aria-label="setting.label" :value="setting.value"
              @update:value="(value) => changeSetting(setting.apply, value)" />
          </div>
        </section>

        <section class="overview-panel">
          <h2>RESG</h2>
          <div class="resg-panel">
            <strong>{{ english ? 'Hextech ARAM' : '海克斯大乱斗' }}</strong>
            <p>{{ english ? 'View builds and augments for the current champion.' : '跟随当前英雄，查看出装与强化。' }}</p>
            <div class="resg-actions">
              <ResgWindowButton overview />
              <NButton size="small" @click="openSettingsModal('multi-window')">
                {{ english ? 'Window settings' : '窗口设置' }}
              </NButton>
            </div>
          </div>
        </section>
      </div>

      <section class="overview-panel" aria-labelledby="league-lookup">
        <h2 id="league-lookup">{{ english ? 'Player lookup' : '玩家查询' }}</h2>
        <NButton type="primary" @click="router.push({ name: 'league-lookup' })">
          {{ english ? 'Open player lookup' : '打开查询' }}
        </NButton>
      </section>

      <footer>
        <NButton size="small" text @click="openSettingsModal('about')">{{ english ? 'About the Toolbox' : '关于工具箱' }}</NButton>
        <LeagueUpdateButton />
      </footer>
    </main>
  </NScrollbar>
</template>

<script setup lang="ts">
import { useInstance } from '@renderer-shared/shards'
import { useAppCommonStore } from '@renderer-shared/shards/app-common/store'
import { AutoGameflowRenderer } from '@renderer-shared/shards/auto-gameflow'
import { useAutoGameflowStore } from '@renderer-shared/shards/auto-gameflow/store'
import { useLeagueClientStore } from '@renderer-shared/shards/league-client/store'
import { WindowManagerRenderer } from '@renderer-shared/shards/window-manager'
import { useAuxWindowStore } from '@renderer-shared/shards/window-manager/store'
import { NButton, NScrollbar, NSwitch, useMessage } from 'naive-ui'
import { computed } from 'vue'
import { useRouter } from 'vue-router'

import ResgWindowButton from '@main-window/components/resg-window/ResgWindowButton.vue'
import { useMainWindowAppContext } from '@main-window/context'
import LeagueUpdateButton from './LeagueUpdateButton.vue'

const client = useLeagueClientStore()
const app = useAppCommonStore()
const autoGameflow = useAutoGameflowStore()
const aux = useAuxWindowStore()
const autoGameflowRenderer = useInstance(AutoGameflowRenderer)
const windows = useInstance(WindowManagerRenderer)
const router = useRouter()
const message = useMessage()
const { openSettingsModal } = useMainWindowAppContext()
const english = computed(() => app.settings.locale.startsWith('en'))

const clientStatus = computed(() => {
  if (client.isConnected) return english.value ? 'League client connected' : '英雄联盟客户端已连接'
  if (client.isConnecting) return english.value ? 'Connecting to League' : '正在连接英雄联盟'
  return english.value ? 'League client disconnected' : '英雄联盟客户端未连接'
})

const phase = computed(() => {
  if (!client.isConnected) return english.value ? 'Start and sign in to the League client.' : '启动英雄联盟客户端并登录。'
  switch (client.gameflow.phase) {
    case 'Lobby': return english.value ? 'In lobby' : '当前在大厅'
    case 'Matchmaking': return english.value ? 'Finding match' : '正在匹配'
    case 'ChampSelect': return english.value ? 'Champion select' : '选人阶段'
    case 'InProgress': return english.value ? 'In a match' : '对局进行中'
    default: return english.value ? 'Ready to use' : '工具箱可用'
  }
})

const quickSettings = computed(() => [
  {
    id: 'accept', label: english.value ? 'Auto-accept matches' : '自动接受对局',
    description: english.value ? 'Confirm when a match is found.' : '找到对局后自动确认。',
    value: autoGameflow.settings.autoAcceptEnabled,
    apply: (value: boolean) => autoGameflowRenderer.setAutoAcceptEnabled(value)
  },
  {
    id: 'aux', label: english.value ? 'Compact companion window' : '小型辅助面板',
    description: english.value ? 'Open quick tools in champion select.' : '选人时显示常用操作。',
    value: aux.settings.enabled,
    apply: (value: boolean) => windows.auxWindow.setEnabled(value)
  }
])

async function changeSetting(apply: (value: boolean) => Promise<unknown>, value: boolean) {
  try {
    await apply(value)
  } catch {
    message.error(english.value ? 'Setting could not be saved.' : '设置未能保存。')
  }
}

</script>

<style scoped>
.league-overview {
  --overview-border: color-mix(in srgb, currentColor 14%, transparent);
  --overview-surface: var(--la-background-color-secondary, var(--la-background-color-primary));
  --overview-accent: #906a21;
  max-width: 1160px;
  margin: 0 auto;
  padding: 20px clamp(18px, 4vw, 42px) 24px;
}
:global([data-theme='dark']) .league-overview { --overview-accent: #d5b56c; }
h1 { margin: 0 0 18px; font-size: 25px; line-height: 1.3; font-weight: 650; }
h2 { margin: 0 0 12px; font-size: 15px; line-height: 1.4; font-weight: 650; }
p { margin: 4px 0 0; opacity: .76; line-height: 1.5; font-size: 12px; }
.client-panel, .overview-panel {
  border: 1px solid var(--overview-border);
  border-radius: 8px;
  padding: 16px;
  background: var(--overview-surface);
}
.client-panel { display: flex; gap: 14px; align-items: center; }
.league-game-mark {
  display: block;
  inline-size: 36px;
  block-size: 36px;
  flex: none;
  background: var(--overview-accent);
  mask: url('./leagueoflegends.svg') center / contain no-repeat;
}
.overview-columns { display: grid; grid-template-columns: 1.25fr 1fr; gap: 16px; margin: 16px 0; }
.settings-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 12px 0; }
.settings-row + .settings-row { border-top: 1px solid var(--overview-border); }
.resg-panel { border-top: 1px solid var(--overview-border); padding-top: 12px; }
.resg-actions, footer { display: flex; gap: 8px; align-items: center; }
.resg-actions { margin-top: 16px; flex-wrap: wrap; }
footer { justify-content: space-between; flex-wrap: wrap; margin: 16px 0 0; }
@media (max-width: 720px) { .overview-columns { grid-template-columns: 1fr; } }
</style>
