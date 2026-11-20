# Keep automatic champion selection local to the standalone ARAM toolbox.
$autoSelectState = 'src\main\shards\auto-select\state.ts'
Update-StandaloneSource $autoSelectState '    return this._akariApiState.autoSelectGroups.groups' @'
    return this._akariApiState.autoSelectGroups.groups.filter(
      (group) => group.groupId === 'aram' && !group.isCustom
    )
'@
Update-StandaloneSource $autoSelectState '  get activeGroupConfig() {' @'
  get activeGroupConfig() {
    if (this.gameMode !== 'ARAM' && this.gameMode !== 'KIWI') return null

'@

# Recheck the current session at execution time for callbacks armed before a queue change.
$autoSelectActions = 'src\main\shards\auto-select\action-executor.ts'
Update-StandaloneSource $autoSelectActions '  championNameWithId(id: number) {' @'
  private get _canActOnCurrentSession() {
    const { state } = this._context
    const session = state.csSession
    const gameId = state.gfSession?.gameData.gameId
    if (!session || session.isSpectating || (gameId && session.gameId && gameId !== session.gameId)) {
      return false
    }

    const config = state.activeGroupConfig
    return Boolean(config && !config.temporarilyDisabled)
  }

  championNameWithId(id: number) {
'@
foreach ($signature in @(
  '  async ban(championId: number, actionId: number, completed: boolean) {',
  '  async pick(championId: number, actionId: number, completed?: boolean) {',
  '  async intent(championId: number, actionId: number) {',
  '  async benchSwap(championId: number) {',
  '  async acceptChampionSwap(tradeId: number) {',
  '  async declineChampionSwap(tradeId: number) {'
)) {
  Update-StandaloneSource $autoSelectActions $signature ($signature + "`n    if (!this._canActOnCurrentSession) return`n")
}

$automationView = Join-Path $renderer 'views\automation\Automation.vue'
Update-StandaloneSource $automationView '    default-tab="auto-gameflow"' '    :default-tab="defaultTab"'
Update-StandaloneSource $automationView "import { computed } from 'vue'" @'
import { computed } from 'vue'
import { useRoute } from 'vue-router'
'@
Update-StandaloneSource $automationView ("import AutoChampConfig from './AutoChampConfig.vue'`n") ''
Update-StandaloneSource $automationView 'import { AiStatus, Cursor1, UserProfile }' 'import { AiStatus, Cursor1 }'
Update-StandaloneSource $automationView @'
  {
    key: 'auto-champ-config',
    name: t('automation.home.autoChampConfig'),
    icon: UserProfile,
    component: AutoChampConfig
  },
'@ ''
Update-StandaloneSource $automationView "])`n</script>" @'
])

const route = useRoute()
const defaultTab = computed(() => {
  const section = route.params.section
  return tabs.value.find((tab) => tab.key === section)?.key ?? 'auto-gameflow'
})
</script>
'@

Update-StandaloneSource (Join-Path $renderer 'routes\index.ts') 'export { router }' @'
router.beforeEach((to) => {
  if (to.name === 'automation' && to.params.section === 'auto-champ-config') {
    return {
      name: 'automation',
      params: { section: 'auto-select' },
      query: to.query,
      hash: to.hash,
      replace: true
    }
  }
})

export { router }
'@

$autoSelectView = Join-Path $renderer 'views\automation\AutoSelect.vue'
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'LeagueAramWindowOptions.vue') -Destination (
  Join-Path $SourceRoot (Split-Path -Parent $autoSelectView)
)
Update-StandaloneSource $autoSelectView '      <div class="mx-auto max-w-[800px] p-6">' @'
      <div class="mx-auto flex max-w-[800px] flex-col gap-4 p-6">
        <LeagueAramWindowOptions />
'@
Update-StandaloneSource $autoSelectView "import AutoSelectEditor from './AutoSelectEditor.vue'" @'
import AutoSelectEditor from './AutoSelectEditor.vue'
import LeagueAramWindowOptions from './LeagueAramWindowOptions.vue'
'@

$autoSelectEditor = Join-Path $renderer 'views\automation\AutoSelectEditor.vue'
Update-StandaloneSource $autoSelectEditor 'ref(''ranked'')' 'ref(''aram'')'
Update-StandaloneSource $autoSelectEditor 'v-if="visibleGroups.length > 0"' 'v-if="visibleGroups.length > 1"'
Update-StandaloneSource $autoSelectEditor 'class="flex h-full items-center justify-center" v-else' 'class="flex h-full items-center justify-center" v-else-if="visibleGroups.length === 0"'

Update-StandaloneSource 'src\shared\i18n\en\renderer\automation.yaml' "  champSelect:`n    title: Auto Pick Ban" "  champSelect:`n    title: ARAM and Hextech ARAM Auto Pick and Ban"
Update-StandaloneSource 'src\shared\i18n\zh-CN\renderer\automation.yaml' "  champSelect:`n    title: 自动英雄选择和禁用" "  champSelect:`n    title: 大乱斗自动选禁"

# Reuse the same RESG snapshot, events, and setting handlers as the settings dialog.
$resgOptions = Join-Path $renderer 'components\resg-window\ResgWindowSettings.vue'
Update-StandaloneSource $resgOptions "const NAMESPACE = 'window-manager-main/resg-window'" @'
defineProps<{ switchesOnly?: boolean }>()

const NAMESPACE = 'window-manager-main/resg-window'
'@
foreach ($label in @('alwaysOnTop.label', 'status.label')) {
  $anchor = "      :label=""t('settings.multiWindow.resgWindow.$label')"""
  Update-StandaloneSource $resgOptions $anchor ('      v-if="!switchesOnly"' + "`n" + $anchor)
}
foreach ($label in @('open', 'close')) {
  $anchor = "    <SettingsRow :label=""t('settings.multiWindow.resgWindow.$label')"""
  $replacement = "    <SettingsRow v-if=""!switchesOnly"" :label=""t('settings.multiWindow.resgWindow.$label')"""
  Update-StandaloneSource $resgOptions $anchor $replacement
}
