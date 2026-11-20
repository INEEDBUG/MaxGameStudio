<template>
  <main class="league-player-search">
    <section v-if="nativeLookup && activeServer" class="search-status">
      <h1>{{ english ? 'Player lookup' : '玩家查询' }}</h1>
      <form class="native-search" @submit.prevent="lookup">
        <NInput
          v-model:value="query"
          :placeholder="english ? 'Riot ID · Name#Tag' : 'Riot ID · 名称#标签'"
          :input-props="{ 'aria-label': english ? 'Riot ID' : '玩家 Riot ID' }"
        />
        <NButton attr-type="submit" type="primary" :loading="pending" :disabled="!query.trim() || pending">
          {{ english ? 'Search' : '查询' }}
        </NButton>
      </form>
      <p v-if="error" class="search-message" role="alert">{{ error }}</p>
      <p v-else-if="searched && !player && !pending" class="search-message" role="status">
        {{ english ? 'No player with that ID was found on this server.' : '当前区服未找到这个 Riot ID。' }}
      </p>
      <div v-if="player" class="player-result" role="status">
        <strong>{{ player.gameName }}<span v-if="player.tagLine"> #{{ player.tagLine }}</span></strong>
        <span>{{ english ? 'Level' : '等级' }} {{ player.summonerLevel }} · {{ player.sgpServerId }}</span>
      </div>
    </section>
    <SearchPane
      v-else-if="activeServer"
      :key="activeServer"
      @navigate-to-summoner="openPlayer"
    />
    <section v-else class="search-status">
      <h1>{{ english ? 'Player lookup' : '玩家查询' }}</h1>
      <p role="status">{{ status }}</p>
    </section>
  </main>
</template>

<script setup lang="ts">
import SearchPane from '@main-window/components/search-pane/SearchPane.vue'
import { PlayerTabsRenderer } from '@main-window/shards/player-tabs'
import { useInstance } from '@renderer-shared/shards'
import { useAppCommonStore } from '@renderer-shared/shards/app-common/store'
import { AkariIpcRenderer } from '@renderer-shared/shards/ipc'
import { useLeagueClientStore } from '@renderer-shared/shards/league-client/store'
import { useSgpStore } from '@renderer-shared/shards/sgp/store'
import { NButton, NInput } from 'naive-ui'
import { computed, onBeforeUnmount, ref, watch } from 'vue'

const client = useLeagueClientStore()
const app = useAppCommonStore()
const sgp = useSgpStore()
const ipc = useInstance(AkariIpcRenderer)
const playerTabs = useInstance(PlayerTabsRenderer)
const { navigateToTabByPuuidAndSgpServerId } = playerTabs.useNavigateToTab()

const english = computed(() => app.settings.locale.startsWith('en'))
const nativeLookup = Boolean((globalThis as typeof globalThis & { __TAURI__?: unknown }).__TAURI__)
const activeServer = computed(() => client.isConnected ? sgp.availability.sgpServerId : '')
const status = computed(() => {
  if (client.isConnected) {
    return nativeLookup
      ? (english.value ? 'Sign in to a League server to search.' : '请登录英雄联盟区服后查询。')
      : (english.value ? 'Reading the connected region…' : '正在读取已连接的大区…')
  }
  if (client.isConnecting) return english.value ? 'Connecting to the League client…' : '正在连接英雄联盟客户端…'
  return english.value
    ? 'Start the League of Legends client and sign in to search for players.'
    : '请启动英雄联盟客户端并登录后查询。国服可通过 WeGame 启动。'
})

type PlayerSummary = {
  gameName: string
  tagLine: string
  sgpServerId: string
  summonerLevel: number
}
const query = ref('')
const pending = ref(false)
const searched = ref(false)
const player = ref<PlayerSummary | null>(null)
const error = ref('')
let lookupId = 0

watch([activeServer, () => client.summoner.me?.puuid], () => {
  lookupId += 1
  pending.value = false
  searched.value = false
  player.value = null
  error.value = ''
})
onBeforeUnmount(() => { lookupId += 1 })

async function lookup() {
  const server = activeServer.value
  const input = query.value.trim()
  if (!server || !input || pending.value) return
  const id = ++lookupId
  pending.value = true
  player.value = null
  error.value = ''
  try {
    const result = await ipc.call<PlayerSummary | null>('player-search-main', 'lookup', server, input)
    if (id === lookupId) {
      player.value = result
      searched.value = true
    }
  } catch (failure) {
    if (id === lookupId) {
      error.value = !english.value && failure instanceof Error
        ? failure.message
        : (english.value ? 'Player lookup failed. Check the Riot ID and try again.' : '玩家查询失败，请核对 Riot ID 后重试。')
    }
  } finally {
    if (id === lookupId) pending.value = false
  }
}

async function openPlayer(puuid: string, sgpServerId: string | null, setCurrent = true) {
  if (!activeServer.value) return
  const server = sgpServerId || activeServer.value
  if (setCurrent) {
    await navigateToTabByPuuidAndSgpServerId(puuid, server)
  } else {
    playerTabs.createTab(puuid, server, { setCurrent: false })
  }
}
</script>

<style scoped>
.league-player-search {
  box-sizing: border-box;
  width: 100%;
  height: 100%;
  min-height: 0;
  max-width: 1160px;
  margin: 0 auto;
  padding: 20px clamp(18px, 4vw, 42px) 24px;
}
.search-status {
  border: 1px solid color-mix(in srgb, currentColor 14%, transparent);
  border-radius: 8px;
  padding: 16px;
  background: var(--la-background-color-secondary, var(--la-background-color-primary));
}
h1 { margin: 0 0 8px; font-size: 25px; line-height: 1.3; font-weight: 650; }
p { margin: 0; font-size: 13px; line-height: 1.5; }
.native-search { display: flex; gap: 8px; max-width: 540px; }
.search-message, .player-result { margin-top: 12px; }
.player-result { display: flex; flex-wrap: wrap; align-items: baseline; gap: 8px 14px; }
.player-result span { opacity: .72; }
</style>
