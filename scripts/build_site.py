"""Build the public, backend-free demo from the current Studio presentation."""
from pathlib import Path
import shutil

root = Path(__file__).resolve().parents[1]
web = root / 'apps/studio/web'
site = root / 'site'
site.mkdir(exist_ok=True)
for name in ('chat.css', 'production.css', 'production.js', 'activity.js'):
    shutil.copyfile(web / name, site / name)
(site / 'embers.js').write_text('// Activity particles are owned by the reactor. No simulated CPU telemetry.\n', encoding='utf-8')
index = site / 'index.html'
html = index.read_text(encoding='utf-8')
if 'src="activity.js"' not in html:
    index.write_text(html.replace('<script defer src="production.js">','<script defer src="activity.js"></script><script defer src="production.js">'), encoding='utf-8')
print('Built static presentation and ember assets in site/')
