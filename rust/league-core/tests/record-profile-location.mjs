import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

const original = readFileSync(
  new URL('../../../packaging/windows/league-standalone/profile-bootstrap.ts', import.meta.url),
  'utf8'
)
const expression = original.match(
  /^function readLocation\(file: string\) \{\r?\n(  return [^\r\n]+)\r?\n\}/m
)
assert.ok(expression, 'The original locator reader changed')
const readLocation = new Function('existsSync', 'readFileSync', 'file', expression[1])
const cases = [
  ['missing', null],
  ['empty', Buffer.from('')],
  ['utf16-with-bom', Buffer.from('\uFEFFX:\\游戏数据\\工具箱', 'utf16le')],
  ['utf16-without-bom', Buffer.from('Y:\\League\\Profile', 'utf16le')],
  ['trailing-whitespace', Buffer.from('\uFEFF Z:\\League Data \u00a0\r\n', 'utf16le')],
  ['only-whitespace', Buffer.from('\uFEFF \r\n\t', 'utf16le')],
  ['odd-byte', Buffer.concat([Buffer.from('X:\\League', 'utf16le'), Buffer.from([0x41])])],
  ['ecmascript-trim', Buffer.from('\u202fX:\\联赛\uFEFF', 'utf16le')],
  ['non-whitespace', Buffer.from('\u0085X:\\League\u0085', 'utf16le')]
]

const recorded = cases.map(([name, bytes]) => ({
  name,
  bytes: bytes && [...bytes],
  path: readLocation(
    () => bytes !== null,
    (_, encoding) => bytes.toString(encoding),
    'synthetic locator'
  ) || null
}))
process.stdout.write('[\n' + recorded.map((value) => '  ' + JSON.stringify(value)).join(',\n') + '\n]\n')
