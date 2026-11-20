<template>
  <main class="league-player-search" :class="{ 'native-page': nativeLookup }">
    <section v-if="nativeSession" class="search-status">
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
      <section v-if="player" class="recent-aram" aria-labelledby="recent-aram-title">
        <h2 id="recent-aram-title">{{ english ? 'Recent ARAM matches' : '近期大乱斗战绩' }}</h2>
        <ol v-if="recentAram?.status === 'ready' && recentAram.matches.length" class="match-list">
          <li v-for="match in recentAram.matches" :key="match.gameId" class="match-row">
            <LcuImage class="match-icon" :src="championIconUri(match.championId)" aria-hidden="true" />
            <div class="match-champion">
              <strong>{{ client.gameData.championName(match.championId) }}</strong>
              <span>{{ match.gameMode === 'KIWI' ? (english ? 'ARAM Mayhem' : '符文大乱斗') : (english ? 'ARAM' : '极地大乱斗') }}</span>
            </div>
            <strong class="match-outcome">{{ outcomeText[match.outcome][english ? 1 : 0] }}</strong>
            <span class="match-score" :aria-label="scoreLabel(match)">{{ match.kills }} / {{ match.deaths }} / {{ match.assists }}</span>
            <div class="match-time">
              <time :datetime="new Date(match.startedAt).toISOString()">{{ matchDate.format(match.startedAt) }}</time>
              <span>{{ durationText(match.durationSeconds) }}</span>
            </div>
          </li>
        </ol>
        <p v-else role="status">
          {{ recentAram?.status === 'ready'
            ? (english ? 'No ARAM matches in the recent record.' : '最近记录中暂无大乱斗战绩。')
            : (english ? 'Recent ARAM matches are unavailable for this player.' : '暂时无法读取该玩家的近期大乱斗战绩。') }}
        </p>
      </section>
    </section>
    <SearchPane
      v-else-if="!nativeLookup && activeServer"
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
import LcuImage from '@renderer-shared/components/LcuImage.vue'
import { useInstance } from '@renderer-shared/shards'
import { useAppCommonStore } from '@renderer-shared/shards/app-common/store'
import { AkariIpcRenderer } from '@renderer-shared/shards/ipc'
import { championIconUri } from '@renderer-shared/shards/league-client/game-data-assets'
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
const nativeSession = computed(() => {
  const auth = client.auth as (NonNullable<typeof client.auth> & { processCreationTime?: string }) | null
  const puuid = client.summoner.me?.puuid
  return nativeLookup && activeServer.value && auth?.pid && auth.processCreationTime && puuid
    ? [auth.pid, auth.processCreationTime, activeServer.value, puuid].join(':')
    : ''
})
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

type RecentMatch = {
  gameId: string
  gameMode: 'ARAM' | 'KIWI'
  startedAt: number
  durationSeconds: number
  championId: number
  outcome: 'win' | 'loss' | 'remake' | 'abort'
  kills: number
  deaths: number
  assists: number
}
type PlayerSummary = {
  gameName: string
  tagLine: string
  sgpServerId: string
  summonerLevel: number
  recentAram?: { status: 'ready'; matches: RecentMatch[] } | { status: 'unavailable' }
}
const query = ref('')
const pending = ref(false)
const searched = ref(false)
const player = ref<PlayerSummary | null>(null)
const error = ref('')
const recentAram = computed(() => player.value?.recentAram)
const matchDate = computed(() => new Intl.DateTimeFormat(english.value ? 'en' : 'zh-CN', {
  month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit'
}))
const outcomeText = {
  win: ['胜利', 'Win'],
  loss: ['失败', 'Loss'],
  remake: ['重开', 'Remake'],
  abort: ['中止', 'Aborted']
}
let lookupId = 0

watch(nativeSession, () => {
  lookupId += 1
  pending.value = false
  searched.value = false
  player.value = null
  error.value = ''
}, { flush: 'sync' })
onBeforeUnmount(() => { lookupId += 1 })

async function lookup() {
  const session = nativeSession.value
  const server = activeServer.value
  const input = query.value.trim()
  if (!session || !input || pending.value) return
  const id = ++lookupId
  const isCurrent = () => id === lookupId && nativeSession.value === session
  pending.value = true
  player.value = null
  error.value = ''
  try {
    const result = await ipc.call<PlayerSummary | null>('player-search-main', 'lookup', server, input)
    if (isCurrent()) {
      player.value = result
      searched.value = true
    }
  } catch (failure) {
    if (isCurrent()) {
      error.value = !english.value && failure instanceof Error
        ? failure.message
        : (english.value ? 'Player lookup failed. Check the Riot ID and try again.' : '玩家查询失败，请核对 Riot ID 后重试。')
    }
  } finally {
    if (isCurrent()) pending.value = false
  }
}

const durationText = (seconds: number) => `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
const scoreLabel = ({ kills, deaths, assists }: RecentMatch) => english.value
  ? `${kills} kills, ${deaths} deaths, ${assists} assists`
  : `击杀 ${kills}，死亡 ${deaths}，助攻 ${assists}`

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
.native-page { overflow-y: auto; }
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
.recent-aram { margin-top: 20px; }
.recent-aram h2 { margin: 0 0 6px; font-size: 16px; font-weight: 650; }
.match-list { margin: 0; padding: 0; list-style: none; }
.match-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px 14px;
  padding: 8px 0;
  border-top: 1px solid color-mix(in srgb, currentColor 12%, transparent);
  font-size: 13px;
}
.match-icon { width: 34px; height: 34px; object-fit: cover; border-radius: 5px; }
.match-champion { display: flex; flex: 1; flex-direction: column; min-width: 110px; }
.match-champion span, .match-time { opacity: .72; }
.match-outcome { min-width: 44px; }
.match-score { min-width: 82px; font-variant-numeric: tabular-nums; }
.match-time { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 0 10px; }
</style>
