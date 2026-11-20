<template>
  <main class="league-player-search">
    <SearchPane
      v-if="activeServer"
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
import { useLeagueClientStore } from '@renderer-shared/shards/league-client/store'
import { useSgpStore } from '@renderer-shared/shards/sgp/store'
import { computed } from 'vue'

const client = useLeagueClientStore()
const app = useAppCommonStore()
const sgp = useSgpStore()
const playerTabs = useInstance(PlayerTabsRenderer)
const { navigateToTabByPuuidAndSgpServerId } = playerTabs.useNavigateToTab()

const english = computed(() => app.settings.locale.startsWith('en'))
const activeServer = computed(() => client.isConnected ? sgp.availability.sgpServerId : '')
const status = computed(() => {
  if (client.isConnected) return english.value ? 'Reading the connected region…' : '正在读取已连接的大区…'
  if (client.isConnecting) return english.value ? 'Connecting to the League client…' : '正在连接英雄联盟客户端…'
  return english.value
    ? 'Start the League of Legends client and sign in to search for players.'
    : '请启动英雄联盟客户端并登录后查询。国服可通过 WeGame 启动。'
})

function openPlayer(puuid: string, sgpServerId: string | null, setCurrent = true) {
  if (!activeServer.value) return
  const server = sgpServerId || activeServer.value
  if (setCurrent) return navigateToTabByPuuidAndSgpServerId(puuid, server)
  playerTabs.createTab(puuid, server, { setCurrent: false })
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
</style>
