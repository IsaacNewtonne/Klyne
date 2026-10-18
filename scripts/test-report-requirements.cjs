const assert = require('assert/strict');
const path = require('path');
const {launch} = require('./test-browser.cjs');

(async () => {
  const browser = await launch();
  try {
    const page = await browser.newPage(), submissions = [], errors = [];
    page.on('pageerror', e => errors.push(e.message));
    const fixture = {id:'123',title:'Report',status:'Completed',messages:[{agent:'Klyne',text:'Saved report.'}],tasks:[],evidence:[],pending:null,execution:{},access:{},provider:{kind:'codex'},result:{outcome:'verified',checks:[{passed:true,outcome:'passed',evidence:'Structural requirements only; source quality is not verified.'}]}};
    await page.route('http://127.0.0.1:4317/**', route => {
      const url = new URL(route.request().url());
      if(!url.pathname.startsWith('/api/')) {
        const file=url.pathname==='/'?'chat.html':url.pathname.slice(1);
        return route.fulfill({path:path.resolve('apps/studio/web',file),contentType:file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':'text/html'});
      }
      if(url.pathname==='/api/chats'&&route.request().method()==='POST') {
        const body=route.request().postDataJSON(); submissions.push(body);
        if(body.contract)fixture.contract=body.contract;
        return route.fulfill({json:{id:'123'}});
      }
      return route.fulfill({json:url.pathname==='/api/chats'?{chats:submissions.length?[fixture]:[]}:url.pathname==='/api/chats/123'?fixture:url.pathname.includes('/pulse')?{seq:0,status:fixture.status}:url.pathname==='/api/apps'?{connections:[]}:{}});
    });
    await page.goto('http://127.0.0.1:4317/');
    await page.waitForFunction(()=>document.querySelector('#connection').textContent==='Connected');
    await page.locator('#settings-button').click();
    await page.locator('#report-settings > summary').click();
    await page.locator('#report-required').check();
    await page.locator('#report-path').fill('research/report.md');
    await page.locator('#report-min-content').fill('250');
    await page.locator('#report-headings').fill('Summary\nFindings\nSources');
    await page.locator('#report-sources').fill('https://example.com/research');
    await page.locator('#settings .dialog-heading button').click();
    await page.locator('#instruction').fill('Research and save my report');
    await page.locator('#send').click();
    await page.waitForFunction(()=>snapshot?.id==='123'&&!submitting);
    assert.deepEqual(submissions[0].contract,{goal:'Research and save my report',criteria:[{ArtifactRequirements:{path:'research/report.md',min_content_chars:250,required_headings:['Summary','Findings','Sources'],source_references:['https://example.com/research']}}]});
    assert.equal(await page.locator('#report-required').isChecked(),false);
    assert.equal(await page.locator('#prod-result-title').textContent(),'Klyne - Report requirements verified');
    await page.locator('#prod-notice').click();
    assert.match(await page.locator('#prod-inspector-body').textContent(),/source quality is not verified/);
    await page.locator('#prod-inspector').evaluate(e=>e.close());

    // Settings for a new task cannot alter the saved contract when resuming.
    fixture.status='Stopped';
    await page.evaluate(()=>{snapshot.status='Stopped';render();document.querySelector('#report-required').checked=true;document.querySelector('#report-headings').value='Weaker';});
    await page.locator('#prod-continue').click();
    await page.waitForFunction(()=>!submitting);
    assert.equal(submissions[1].resume,true);
    assert.equal(submissions[1].contract,undefined);

    // A new conversational message must not inherit report requirements.
    await page.locator('#new-chat').click();
    assert.equal(await page.locator('#report-required').isChecked(),false);
    await page.locator('#instruction').fill('Hello');
    await page.locator('#send').click();
    await page.waitForFunction(()=>!submitting);
    assert.equal(submissions[2].contract,undefined);
    assert.deepEqual(errors,[]);
    console.log('Report requirements: form submission, result limits, immutable resume and ordinary conversation passed');
  } finally {await browser.close();}
})().catch(error=>{console.error(error);process.exit(1);});
