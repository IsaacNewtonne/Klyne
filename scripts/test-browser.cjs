const fs = require('fs');
const path = require('path');
const {chromium} = require('playwright');

// Keep test profiles/artifacts in the checkout. CI installs the matching
// Playwright browser; local runs can use an installed Chrome via CHROME_BINARY.
process.env.TEMP = process.env.TMP = path.resolve('workspace/ui-test-temp');
fs.mkdirSync(process.env.TEMP, {recursive:true});
const chrome = process.env.CHROME_BINARY ||
  (process.platform === 'win32' && fs.existsSync('C:/Program Files/Google/Chrome/Application/chrome.exe')
    ? 'C:/Program Files/Google/Chrome/Application/chrome.exe' : undefined);
module.exports.launch = () => chromium.launch({headless:true, ...(chrome ? {executablePath:chrome} : {})});
