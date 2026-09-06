<template>
  <SettingsSection :title="t('settings.multiWindow.resgWindow.title')">
    <SettingsRow
      :label="t('settings.multiWindow.resgWindow.enabled.label')"
      :label-description="t('settings.multiWindow.resgWindow.enabled.description')"
      :label-width="400"
    >
      <NSwitch
        size="small"
        :value="snapshot.enabled"
        :loading="busy"
        :disabled="busy"
        @update:value="(value) => updateEnabled(value)"
      />
    </SettingsRow>
    <SettingsRow
      :label="t('settings.multiWindow.resgWindow.autoShow.label')"
      :label-description="t('settings.multiWindow.resgWindow.autoShow.description')"
      :label-width="400"
    >
      <NSwitch
        size="small"
        :value="snapshot.autoShow"
        :loading="busy"
        :disabled="busy || !snapshot.enabled"
        @update:value="(value) => updateAutoShow(value)"
      />
    </SettingsRow>
    <SettingsRow
      :label="t('settings.multiWindow.resgWindow.status.label')"
      :label-description="t('settings.multiWindow.resgWindow.status.description')"
      :label-width="400"
    >
      <NFlex align="center" class="max-w-full justify-end gap-2">
        <NTag size="small" :type="statusType">{{ statusLabel }}</NTag>
        <NButton
          v-if="snapshot.status === 'error'"
          size="small"
          secondary
          :loading="busy"
          @click="retry"
        >
          {{ t('settings.multiWindow.resgWindow.retry') }}
        </NButton>
      </NFlex>
    </SettingsRow>
    <SettingsRow :label="t('settings.multiWindow.resgWindow.open')" :label-width="400">
      <NButton
        size="small"
        type="primary"
        secondary
        :loading="busy"
        :disabled="busy || !snapshot.enabled || !snapshot.available"
        @click="show"
      >
        {{ t('settings.multiWindow.resgWindow.open') }}
      </NButton>
    </SettingsRow>
    <SettingsRow :label="t('settings.multiWindow.resgWindow.close')" :label-width="400">
      <NButton
        size="small"
        secondary
        :loading="busy"
        :disabled="busy || !snapshot.visible"
        @click="close"
      >
        {{ t('settings.multiWindow.resgWindow.close') }}
      </NButton>
    </SettingsRow>
    <div
      v-if="snapshot.error"
      role="alert"
      class="text-right text-xs text-red-600 dark:text-red-300"
    >
      {{ t('settings.multiWindow.resgWindow.genericError') }}
    </div>
    <div class="text-right text-xs text-neutral-500 dark:text-neutral-400">
      {{ t('settings.multiWindow.resgWindow.scope') }}
    </div>
  </SettingsSection>
</template>

<script setup lang="ts">
import SettingsRow from '@renderer-shared/components/SettingsRow.vue'
import SettingsSection from '@renderer-shared/components/SettingsSection.vue'
import { useInstance } from '@renderer-shared/shards'
import { AkariIpcRenderer } from '@renderer-shared/shards/ipc'
import { useTranslation } from 'i18next-vue'
import { NButton, NFlex, NSwitch, NTag, useMessage } from 'naive-ui'
import { computed, onMounted, reactive, ref } from 'vue'

type ResgSnapshot = {
  enabled: boolean
  autoShow: boolean
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
const revision = ref(0)
let requestId = 0
const snapshot = reactive<ResgSnapshot>({
  enabled: false,
  autoShow: true,
  visible: false,
  available: false,
  championId: 0,
  status: 'disabled',
  error: null
})

const applySnapshot = (next: ResgSnapshot) => Object.assign(snapshot, next)
const statusLabel = computed(() => t(`settings.multiWindow.resgWindow.status.${snapshot.status}`))
const statusType = computed(() =>
  snapshot.status === 'error' ? 'error' : snapshot.status === 'ready' ? 'success' : 'default'
)

const call = async (name: string, ...args: unknown[]) => {
  if (busy.value) return
  const currentRequestId = ++requestId
  const requestRevision = revision.value
  busy.value = true
  try {
    const next = await ipc.call<ResgSnapshot>(NAMESPACE, name, ...args)
    if (currentRequestId === requestId && requestRevision === revision.value) {
      applySnapshot(next)
    }
  } catch {
    message.error(t('settings.multiWindow.resgWindow.genericError'))
  } finally {
    busy.value = false
  }
}

const updateEnabled = (value: boolean) => call('setEnabled', value)
const updateAutoShow = (value: boolean) => call('setAutoShow', value)
const retry = () => call('retry')
const show = () => call('show')
const close = () => call('close')

ipc.onEventVue(NAMESPACE, 'changed', (next: ResgSnapshot) => {
  revision.value += 1
  applySnapshot(next)
})

onMounted(async () => {
  const requestRevision = revision.value
  const currentRequestId = ++requestId
  try {
    const next = await ipc.call<ResgSnapshot>(NAMESPACE, 'getSnapshot')
    if (currentRequestId === requestId && revision.value === requestRevision) applySnapshot(next)
  } catch {
    message.error(t('settings.multiWindow.resgWindow.genericError'))
  }
})
</script>
