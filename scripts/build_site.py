"""Build the public, backend-free demo from the current Studio presentation."""
from pathlib import Path
import shutil

root = Path(__file__).resolve().parents[1]
web = root / 'apps/studio/web'
site = root / 'site'
site.mkdir(exist_ok=True)
for name in ('chat.css', 'production.css', 'production.js'):
    shutil.copyfile(web / name, site / name)
source = (web / 'chat.js').read_text(encoding='utf-8')
embers = source[source.index('// Ambient embers:'):]
start = embers.index('  let loadTimer')
end = embers.index('  const pointer', start)
embers = embers[:start] + embers[end:]
assert '/api/' not in embers and 'fetch(' not in embers
(site / 'embers.js').write_text(embers, encoding='utf-8')
print('Built static presentation and ember assets in site/')
