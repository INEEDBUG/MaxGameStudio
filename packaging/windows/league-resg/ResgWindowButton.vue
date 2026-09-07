<template>
  <NTooltip :z-index="75000">
    <template #trigger>
      <div
        class="resg-button-outer"
        :class="{ 'pointer-events-none opacity-45': !canShow || busy }"
        :aria-disabled="!canShow"
        :aria-busy="busy"
        role="button"
        tabindex="0"
        @click="show"
        @keydown.enter="show"
        @keydown.space.prevent="show"
      >
        <div class="resg-button-inner text-[11px] font-semibold tracking-tight">
          <NSpin v-if="busy" :size="14" />
          <span v-else>RESG</span>
        </div>
      </div>
    </template>
    {{ canShow ? t('titlebar.actions.resgWindow') : t('titlebar.actions.resgUnavailable') }}
  </NTooltip>
</template>

<script setup lang="ts">
import { useInstance } from '@renderer-shared/shards'
import { AkariIpcRenderer } from '@renderer-shared/shards/ipc'
import { useTranslation } from 'i18next-vue'
import { NSpin, NTooltip, useMessage } from 'naive-ui'
import { computed, onMounted, reactive, ref } from 'vue'

type ResgSnapshot = {
  enabled: boolean
  autoShow: boolean
  alwaysOnTop: boolean
  visible: boolean
  available: boolean
  championId: number
  status: 'disabled' | 'waiting' | 'loading' | 'ready' | 'error'
  error: string | null
}

const NAMESPACE = 'window-manager-main/resg-window'
const ipc = useInstance(AkariIpcRenderer)
const { t } = useTranslation()
const message = useMessage()
const busy = ref(false)
const snapshot = reactive<ResgSnapshot>({
  enabled: false,
  autoShow: true,
  alwaysOnTop: false,
  visible: false,
  available: false,
  championId: 0,
  status: 'disabled',
  error: null
})
const revision = ref(0)
let requestId = 0
const canShow = computed(() => snapshot.enabled && snapshot.available)

const show = async () => {
  if (!canShow.value || busy.value) return
  const currentRequestId = ++requestId
  const requestRevision = revision.value
  busy.value = true
  try {
    const next = await ipc.call<ResgSnapshot>(NAMESPACE, 'show')
    if (currentRequestId === requestId && requestRevision === revision.value) {
      Object.assign(snapshot, next)
    }
  } catch {
    message.error(t('settings.multiWindow.resgWindow.genericError'))
  } finally {
    busy.value = false
  }
}

ipc.onEventVue(NAMESPACE, 'changed', (next: ResgSnapshot) => {
  revision.value += 1
  Object.assign(snapshot, next)
})

onMounted(async () => {
  const requestRevision = revision.value
  const currentRequestId = ++requestId
  try {
    const next = await ipc.call<ResgSnapshot>(NAMESPACE, 'getSnapshot')
    if (currentRequestId === requestId && revision.value === requestRevision) {
      Object.assign(snapshot, next)
    }
  } catch {
    // Main may not have registered the optional controller in a local POC.
  }
})
</script>

<style scoped>
@reference '@renderer-shared/assets/css/tailwind.css';

.resg-button-outer {
  display: flex;
  justify-content: center;
  align-items: center;
  width: 32px;
  height: 100%;
  cursor: pointer;
  -webkit-app-region: no-drag;
}

.resg-button-inner {
  padding: 4px;
  border-radius: 2px;
  color: var(--la-color-text-primary);
  transition:
    background-color 0.3s,
    color 0.3s;
}

.resg-button-outer:hover .resg-button-inner {
  background-color: color-mix(in srgb, var(--la-color-text-primary) 15%, transparent);
}

</style>
