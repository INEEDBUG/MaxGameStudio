<template>
  <div class="league-aram-window-options grid grid-cols-1 gap-4 md:grid-cols-2">
    <SettingsSection :title="t('settings.multiWindow.opggWindow.title')">
      <SettingsRow :label="t('settings.multiWindow.opggWindow.enabled.label')">
        <NSwitch
          size="small"
          :aria-label="t('settings.multiWindow.opggWindow.enabled.label')"
          :value="opgg.settings.enabled"
          @update:value="(value) => windows.opggWindow.setEnabled(value)"
        />
      </SettingsRow>
      <SettingsRow :label="t('settings.multiWindow.opggWindow.autoShow.label')">
        <NSwitch
          size="small"
          :aria-label="t('settings.multiWindow.opggWindow.autoShow.label')"
          :value="opgg.settings.autoShow"
          :disabled="!opgg.settings.enabled"
          @update:value="(value) => windows.opggWindow.setAutoShow(value)"
        />
      </SettingsRow>
    </SettingsSection>
    <ResgWindowSettings switches-only />
  </div>
</template>

<script setup lang="ts">
import ResgWindowSettings from '@main-window/components/resg-window/ResgWindowSettings.vue'
import SettingsRow from '@renderer-shared/components/SettingsRow.vue'
import SettingsSection from '@renderer-shared/components/SettingsSection.vue'
import { useInstance } from '@renderer-shared/shards'
import { WindowManagerRenderer } from '@renderer-shared/shards/window-manager'
import { useOpggWindowStore } from '@renderer-shared/shards/window-manager/store'
import { useTranslation } from 'i18next-vue'
import { NSwitch } from 'naive-ui'

const { t } = useTranslation()
const windows = useInstance(WindowManagerRenderer)
const opgg = useOpggWindowStore()
</script>
