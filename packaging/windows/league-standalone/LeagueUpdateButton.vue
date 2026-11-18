<template>
  <div class="league-update" aria-live="polite">
    <NButton size="small" :loading="status === 'checking'" @click="check()">
      {{ english ? 'Check for Toolbox updates' : '检查工具箱更新' }}
    </NButton>
    <NButton v-if="update" size="small" type="primary" tag="a" :href="update.url"
      target="_blank" rel="noopener noreferrer">
      {{ (english ? 'Download ' : '下载 ') + update.version }}
    </NButton>
    <span v-else-if="status === 'current'">{{ english ? 'Up to date' : '已是最新版本' }}</span>
    <span v-else-if="status === 'error'">{{ english ? 'Update check failed' : '暂时无法检查更新' }}</span>
  </div>
</template>

<script lang="ts">
import { ref, shallowRef } from 'vue'
import { leagueUpdate, leagueUpdateFeed } from './league-update'

const status = ref<'idle' | 'checking' | 'current' | 'available' | 'error'>('idle')
const update = shallowRef<ReturnType<typeof leagueUpdate>>(null)
let autoChecked = false
</script>

<script setup lang="ts">
import { computed, onMounted } from 'vue'
import { NButton } from 'naive-ui'
import { useAppCommonStore } from '@renderer-shared/shards/app-common/store'
import { useSelfUpdateStore } from '@renderer-shared/shards/self-update/store'

const app = useAppCommonStore()
const preferences = useSelfUpdateStore()
const english = computed(() => app.settings.locale.startsWith('en'))

async function check(manual = true) {
  if (status.value === 'checking') return
  status.value = 'checking'
  try {
    const response = await fetch(leagueUpdateFeed(app.version), {
      cache: 'no-store', credentials: 'omit', redirect: 'error', signal: AbortSignal.timeout(6000)
    })
    if (!response.ok) throw new Error('League manifest unavailable')
    update.value = leagueUpdate(await response.json(), app.version)
    status.value = update.value ? 'available' : manual ? 'current' : 'idle'
  } catch {
    update.value = null
    status.value = manual ? 'error' : 'idle'
  }
}

onMounted(() => {
  if (!autoChecked && preferences.settings.autoCheckUpdates) {
    autoChecked = true
    void check(false)
  }
})
</script>

<style scoped>
.league-update { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; font-size: 12px; }
</style>
