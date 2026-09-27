// Node 侧复现线上 LaTeX 模式编译 transformer 源
import * as fs from 'node:fs'
import * as path from 'node:path'
import { fileURLToPath } from 'node:url'
const dir = path.dirname(fileURLToPath(import.meta.url))
process.chdir('/home/ubuntu/NTex/samples/transformer-real')

const { default: init, set_bundle, set_latex_mode, compile_document } =
  await import(path.join(dir, 'ntex_wasm.js'))
await init()

const bundle = fs.readFileSync('/home/ubuntu/NTex/site/public/latex-bundle.bin')
set_bundle(new Uint8Array(bundle))
set_latex_mode(true)

const tex = fs.readFileSync('transformer-standalone.tex', 'utf8')
try {
  const doc = compile_document(tex)
  console.log('pages:', doc.page_count())
  console.log('used fonts:', doc.used_fonts().slice(0, 5))
} catch (e) {
  console.log('COMPILE ERROR:', String(e).slice(0, 600))
}
