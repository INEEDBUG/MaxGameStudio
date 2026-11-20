<template>
  <section class="opgg-companion">
    <template v-if="draft">
      <div class="guide-heading">
        <LcuImage
          v-if="draft.championId"
          :src="championIconUri(draft.championId)"
          class="champion-icon"
          aria-hidden="true"
        />
        <div>
          <p class="mode">{{ modeName }}</p>
          <h1>{{ championName }}</h1>
        </div>
      </div>
      <NButton type="primary" :loading="opening" @click="openGuide">
        {{ english ? 'Open OP.GG in browser' : '在浏览器打开 OP.GG' }}
      </NButton>
      <p v-if="error" role="alert">{{ error }}</p>
    </template>
    <p v-else role="status">
      {{ error || (english ? 'Waiting for an active ARAM champion select.' : '等待进入大乱斗选人。') }}
    </p>
  </section>
</template>

<script setup lang="ts">
import LcuImage from '@renderer-shared/components/LcuImage.vue'
import { useInstance } from '@renderer-shared/shards'
import { useAppCommonStore } from '@renderer-shared/shards/app-common/store'
import { AkariIpcRenderer } from '@renderer-shared/shards/ipc'
import { championIconUri } from '@renderer-shared/shards/league-client/game-data-assets'
import { useLeagueClientStore } from '@renderer-shared/shards/league-client/store'
import { PiniaMobxUtilsRenderer } from '@renderer-shared/shards/pinia-mobx-utils'
import { useOpggWindowStore } from '@renderer-shared/shards/window-manager/store'
import { NButton } from 'naive-ui'
import { computed, onMounted, ref, shallowReactive } from 'vue'

type Draft = { gameMode: 'ARAM' | 'KIWI'; championId: number | null }

const namespace = 'opgg-guide-main'
const ipc = useInstance(AkariIpcRenderer)
const sync = useInstance(PiniaMobxUtilsRenderer)
const client = useLeagueClientStore()
const app = useAppCommonStore()
const windowStore = useOpggWindowStore()
const state = shallowReactive<{ draft: Draft | null }>({ draft: null })
const draft = computed(() => windowStore.settings.enabled ? state.draft : null)
const english = computed(() => app.settings.locale.startsWith('en'))
const modeName = computed(() => draft.value?.gameMode === 'KIWI'
  ? (english.value ? 'ARAM Mayhem' : '符文大乱斗')
  : (english.value ? 'ARAM' : '极地大乱斗'))
const championName = computed(() => draft.value?.championId
  ? client.gameData.championName(draft.value.championId)
  : (english.value ? 'Champion guide' : '英雄攻略'))
const opening = ref(false)
const error = ref('')

onMounted(async () => {
  try {
    await sync.sync(namespace, 'state', state)
  } catch {
    error.value = english.value ? 'The current draft is unavailable.' : '暂时无法读取当前选人状态。'
  }
})

async function openGuide() {
  if (!draft.value || opening.value) return
  opening.value = true
  error.value = ''
  try {
    await ipc.call(namespace, 'openGuide')
  } catch {
    error.value = english.value ? 'Could not open the guide for the current draft.' : '暂时无法打开当前选人的攻略。'
  } finally {
    opening.value = false
  }
}
</script>

<style scoped>
.opgg-companion { padding: 24px; overflow-y: auto; }
.guide-heading { display: flex; align-items: center; gap: 12px; margin-bottom: 18px; }
.champion-icon { width: 48px; height: 48px; border-radius: 8px; object-fit: cover; }
h1 { margin: 0; font-size: 22px; line-height: 1.3; font-weight: 650; }
p { margin: 0; font-size: 13px; }
.mode { opacity: .72; margin-bottom: 3px; }
[role='alert'] { margin-top: 10px; }
</style>
