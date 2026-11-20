& {
  $bootstrap = 'src\main\bootstrap\index.ts'
  Update-StandaloneSource $bootstrap "import { RespawnTimerMain } from '@main/shards/respawn-timer'`n" ''
  Update-StandaloneSource $bootstrap "    manager.use(RespawnTimerMain)`n" ''

  $mainShards = Join-Path $renderer 'shards\index.ts'
  Update-StandaloneSource $mainShards "import { RespawnTimerRenderer } from '@renderer-shared/shards/respawn-timer'`n" ''
  Update-StandaloneSource $mainShards "manager.use(RespawnTimerRenderer)`n" ''

  $sidebar = Join-Path $renderer 'components\sidebar\SidebarFixed.vue'
  $respawnIndicator = @'
    <!-- respawn timer -->
    <RespawnTimerItem
      v-if="rts.settings.enabled && rts.info.isDead"
      :time-left="rts.info.timeLeft"
      :total-time="rts.info.totalTime"
      :is-collapsed="isCollapsed"
    />
'@
  Update-StandaloneSource $sidebar ($respawnIndicator + "`n`n") ''
  Update-StandaloneSource $sidebar "import { useRespawnTimerStore } from '@renderer-shared/shards/respawn-timer/store'`n" ''
  Update-StandaloneSource $sidebar "import { RespawnTimerItem } from './respawn-timer-item'`n" ''
  Update-StandaloneSource $sidebar "const rts = useRespawnTimerStore()`n" ''

  $respawnNotification = @'
const notification = useNotification()

watch(
  () => rts.info.isDead,
  (isDead, prevIsDead) => {
    if (!isDead && prevIsDead && mws.focus === 'focused') {
      notification.success({
        title: () => t('navigation.sidebar.status.respawned'),
        content: () => t('navigation.sidebar.status.respawnedContent'),
        duration: 4000
      })
    }
  }
)
'@
  Update-StandaloneSource $sidebar $respawnNotification ''
  Update-StandaloneSource $sidebar 'NProgress, NTooltip, useNotification' 'NProgress, NTooltip'
  Update-StandaloneSource $sidebar "import { useMainWindowStore } from '@renderer-shared/shards/window-manager/store'`n" ''
  Update-StandaloneSource $sidebar "const mws = useMainWindowStore()`n" ''

  $settings = Join-Path $renderer 'components\settings-modal\SettingsModal.vue'
  $miscTab = @'
      <NTabPane name="misc" :tab="t('settings.misc.title')">
        <template #tab>
          <div class="tab-icon-title">
            <NIcon class="icon"><ToolFilledIcon /> </NIcon>
            <span>{{ t('settings.misc.title') }}</span>
          </div>
        </template>
        <MiscSettings />
      </NTabPane>
'@
  Update-StandaloneSource $settings ($miscTab + "`n") ''
  Update-StandaloneSource $settings "import { ToolFilled as ToolFilledIcon } from '@vicons/antd'`n" ''
  Update-StandaloneSource $settings "import MiscSettings from './MiscSettings.vue'`n" ''

  $notifications = Join-Path $renderer 'shards\simple-notifications\index.ts'
  Update-StandaloneSource $notifications "import { setupStreamerModeNotifications } from './streamer-mode-notifications'`n" ''
  Update-StandaloneSource $notifications "    this.setupInAppScope.addSetupFn(() => setupStreamerModeNotifications(this.context))`n" ''
  foreach ($anchor in @(
    '  LAST_DISMISS_LIVE_STREAMING_STREAMER_MODE_SETTING_KEY,',
    '  NEVER_SHOW_LIVE_STREAMING_STREAMER_MODE_SETTING_KEY,',
    '  static NEVER_SHOW_SETTING_KEY = NEVER_SHOW_LIVE_STREAMING_STREAMER_MODE_SETTING_KEY',
    '  static LAST_DISMISS_SETTING_KEY = LAST_DISMISS_LIVE_STREAMING_STREAMER_MODE_SETTING_KEY'
  )) {
    Update-StandaloneSource $notifications ($anchor + "`n") ''
  }

  # Settings restoration assigns properties directly. Keep the registered keys
  # and ignore restored values at the state boundary so loading leaves disk intact.
  $appSettings = 'src\main\shards\app-common\state.ts'
  $windowSettings = 'src\main\shards\window-manager\state.ts'
  foreach ($setting in @(
    @{ Path = $appSettings; Name = 'streamerMode' },
    @{ Path = $appSettings; Name = 'streamerModeUseAkariStyledName' },
    @{ Path = $windowSettings; Name = 'contentProtection' }
  )) {
    $name = $setting.Name
    $disabledProperty = @"
  get $name(): boolean {
    return false
  }

  set $name(_savedValue: boolean) {
    // Retain assignment compatibility with saved profiles and the settings service.
  }
"@
    Update-StandaloneSource $setting.Path "  ${name}: boolean = false" $disabledProperty
  }
}
