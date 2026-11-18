import assert from 'node:assert/strict'
import { test } from 'node:test'
import { companionWindowDemand } from './companion-window-demand.ts'

const base = {
  enabled: true,
  autoShow: true,
  initialized: true,
  connected: true,
  ready: false,
  phase: 'ChampSelect',
  spectating: false
}

test('enabled defaults create neither companion renderer without a connected client', () => {
  for (const kind of ['aux', 'opgg']) {
    assert.equal(companionWindowDemand(kind, { ...base, connected: false }), 'ignore')
    assert.equal(companionWindowDemand(kind, { ...base, enabled: false }), 'ignore')
    assert.equal(companionWindowDemand(kind, { ...base, autoShow: false }), 'ignore')
    assert.equal(companionWindowDemand(kind, { ...base, initialized: false }), 'ignore')
  }
})

test('each eligible phase prepares the window and displays it after readiness', () => {
  for (const kind of ['aux', 'opgg']) {
    assert.equal(companionWindowDemand(kind, base), 'prepare')
    assert.equal(companionWindowDemand(kind, { ...base, ready: true }), 'show')
  }
  for (const phase of ['Lobby', 'Matchmaking', 'ReadyCheck']) {
    assert.equal(companionWindowDemand('aux', { ...base, phase }), 'prepare')
    assert.equal(companionWindowDemand('opgg', { ...base, phase }), 'ignore')
  }
  assert.equal(companionWindowDemand('aux', { ...base, spectating: true }), 'ignore')
  assert.equal(companionWindowDemand('opgg', { ...base, spectating: true }), 'prepare')
})

test('manual opening outside an auto-show phase does not change the automatic decision', () => {
  for (const ready of [false, true]) {
    assert.equal(companionWindowDemand('aux', { ...base, ready, phase: 'None' }), 'hide')
    assert.equal(companionWindowDemand('opgg', { ...base, ready, phase: 'None' }), 'ignore')
  }
})
