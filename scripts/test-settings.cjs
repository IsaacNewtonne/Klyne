const assert = require('assert/strict');
const fs = require('fs');
const path = require('path');
const {launch} = require('./test-browser.cjs');

(async () => {
  const browser = await launch();
  try {
    const page = await browser.newPage(), errors = [], submissions = [], scheduleRequests = [];
    page.on('pageerror', error => errors.push(error.message));
    const fixture = {
      id:'123',title:'Previous project',status:'Stopped',messages:[],tasks:[],evidence:[],pending:null,
      activity:null,activity_events:[],used:0,limit:25,workspace:'C:\\projects\\previous-project',prompt_maker:true,
      provider:{kind:'codex',endpoint:'',model:''},access:{web:true,terminal:true,desktop:false,apps:true},
      execution:{command_policy:'ask',max_tokens:5000,max_cost_usd:2,timeout_seconds:30,max_review_rounds:2}
    };
    await page.route('http://127.0.0.1:4317/**', route => {
      const url = new URL(route.request().url());
      if (!url.pathname.startsWith('/api/')) {
        const file=url.pathname==='/'?'chat.html':url.pathname.slice(1);
        const local=path.resolve('apps/studio/web',file);
        assert(fs.existsSync(local), file);
        return route.fulfill({path:local,contentType:file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':'text/html'});
      }
      if (url.pathname==='/api/connections/check') return route.fulfill({json:{message:'Ollama is online.',models:['tiger-gemma-12b-v3:IQ4_XS','another-model:latest']}});
      if (url.pathname==='/api/schedules') {
        if(route.request().method()==='POST') scheduleRequests.push(route.request().postDataJSON());
        return route.fulfill({json:{jobs:[],id:'scheduled-fixture'}});
      }
      if (url.pathname==='/api/chats' && route.request().method()==='POST') {
        submissions.push(route.request().postDataJSON());
        // Record the actual user-form payload; never call a model or change a project.
        return route.fulfill({status:400,json:{error:'Captured test submission'}});
      }
      return route.fulfill({json:url.pathname==='/api/chats'?{chats:[fixture]}:
        url.pathname==='/api/chats/123'?fixture:
        url.pathname.includes('/pulse')?{seq:0,status:fixture.status}:
        url.pathname==='/api/apps'?{connections:[]}:{}});
    });
    const ready = () => page.waitForFunction(() => document.querySelector('#connection').textContent==='Connected');
    const state = () => page.evaluate(() => ({
      workspace:document.querySelector('#host-workspace').value,
      access:Object.fromEntries(['web','terminal','desktop','apps'].map(k=>[k,document.querySelector('#access-'+k).checked])),
      execution:executionConfig(),mode:document.querySelector('#writing-mode').value
    }));
    await page.goto('http://127.0.0.1:4317/');await ready();
    await page.locator('.chat-item[data-chat="123"]').click();
    await page.waitForFunction(() => snapshot?.id==='123');
    assert.equal((await state()).workspace,fixture.workspace);
    await page.locator('#new-chat').click();
    assert.deepEqual(await state(),{workspace:'',access:{web:false,terminal:false,desktop:false,apps:false},execution:{max_steps:0,timeout_seconds:0,max_review_rounds:0,command_policy:'ask',max_tokens:0,max_cost_usd:0},mode:'general'});
    await page.locator('#instruction').fill('Create a fresh workspace file');
    await page.locator('#send').click();
    await page.waitForFunction(() => document.querySelector('#form-error').textContent==='Captured test submission');
    assert.equal(submissions[0].id,null);
    assert.equal(submissions[0].workspace,'');
    assert.deepEqual(submissions[0].access,{web:false,terminal:false,apps:false,desktop:false});
    assert.equal(submissions[0].execution.max_steps,0);

    // Disabling Terminal must not submit an old project selection.
    await page.locator('.chat-item[data-chat="123"]').click();
    await page.waitForFunction(() => snapshot?.id==='123');
    await page.locator('#settings-button').click();
    await page.locator('#execution-settings > summary').click();
    await page.locator('#access-terminal').uncheck();
    await page.locator('#settings .dialog-heading button').click();
    await page.locator('#instruction').fill('Answer without terminal access');
    await page.locator('#send').click();
    await page.waitForFunction(() => document.querySelector('#form-error').textContent==='Captured test submission');
    assert.equal(submissions[1].workspace,'');
    assert.equal(submissions[1].access.terminal,false);

    await page.locator('#new-chat').click();
    await page.locator('#settings-button').click();
    await page.locator('#trusted-laptop').click();
    await page.locator('#settings .dialog-heading button').click();
    await page.reload();await ready();
    assert.equal((await state()).execution.command_policy,'autonomous');
    assert.deepEqual((await state()).access,{web:true,terminal:true,desktop:true,apps:true});
    // Saved conversation policy takes precedence over the global new-chat default,
    // including when loaded through the share/recovery URL rather than a list click.
    await page.goto('http://127.0.0.1:4317/?restore=1#chat=123');await ready();
    await page.waitForFunction(() => snapshot?.id==='123');
    assert.equal((await state()).execution.command_policy,'ask');
    assert.deepEqual((await state()).access,fixture.access);
    await page.locator('#new-chat').click();
    assert.equal((await state()).execution.command_policy,'autonomous');
    assert.equal((await state()).workspace,'');
    assert.equal((await state()).execution.timeout_seconds,0);
    // A stale typed name must not hide the installed choices; resume uses the explicit selection.
    fixture.status='Blocked';
    await page.locator('.chat-item[data-chat="123"]').click();
    await page.waitForFunction(() => snapshot?.id==='123');
    await page.locator('#settings-button').click();
    await page.locator('#provider-kind').selectOption('ollama');
    await page.locator('#provider-model').fill('missing-old-model');
    await page.locator('#check-provider').click();
    await page.locator('#available-models').waitFor({state:'visible'});
    assert.match(await page.locator('#provider-status').textContent(),/current model was not found/);
    assert.equal(await page.locator('#provider-model').inputValue(),'missing-old-model');
    assert.deepEqual(await page.locator('#available-models option').allTextContents(),['Choose a model','tiger-gemma-12b-v3:IQ4_XS','another-model:latest']);
    await page.locator('#available-models').selectOption('tiger-gemma-12b-v3:IQ4_XS');
    assert.equal(await page.locator('#provider-model').inputValue(),'tiger-gemma-12b-v3:IQ4_XS');
    await page.locator('#settings .dialog-heading button').click();
    await page.evaluate(() => document.querySelector('#resume-chat').click());
    await page.waitForFunction(() => !submitting);
    assert.equal(submissions.at(-1).resume,true);
    assert.equal(submissions.at(-1).provider.model,'tiger-gemma-12b-v3:IQ4_XS');
    // Reviewer settings are sent independently and restored for new chats.
    await page.locator('#new-chat').click();
    await page.locator('#settings-button').click();
    await page.locator('#review-settings > summary').click();
    await page.locator('#reviewer-kind').selectOption('ollama');
    await page.locator('#reviewer-model').fill('review-model');
    await page.locator('#check-reviewer').click();
    await page.waitForFunction(() => document.querySelector('#reviewer-status').textContent==='Ollama is online.');
    await page.locator('#settings .dialog-heading button').click();
    await page.reload();await ready();
    assert.equal(await page.locator('#reviewer-model').inputValue(),'review-model');
    await page.locator('#instruction').fill('Inspect my files');
    await page.locator('#send').click();
    await page.waitForFunction(() => document.querySelector('#form-error').textContent==='Captured test submission');
    assert.equal(submissions.at(-1).reviewer.model,'review-model');
    await page.locator('#settings-button').click();
    await page.locator('#intelligence-settings > summary').click();
    await page.locator('#decisions-enabled').check();
    await page.locator('#learn-procedures').check();
    await page.locator('#decision-specialist').check();
    await page.locator('#decision-price').fill('0.25');
    await page.locator('#decision-price').press('Tab');
    await page.locator('#settings .dialog-heading button').click();
    await page.reload();await ready();
    assert.equal(await page.locator('#decisions-enabled').isChecked(),true);
    await page.locator('#instruction').fill('Route this task');
    await page.locator('#send').click();
    await page.waitForFunction(() => document.querySelector('#form-error').textContent==='Captured test submission');
    assert.equal(submissions.at(-1).decisions.specialist.model,'review-model');
    assert.equal(submissions.at(-1).decisions.input_usd_per_million,0.25);
    assert.equal(submissions.at(-1).learn_procedures,true);
    // Existing conversations use their saved reviewer, not the new-chat default.
    fixture.execution.reviewer={kind:'ollama',endpoint:'http://127.0.0.1:11434',model:'saved-reviewer'};
    fixture.status='Completed';fixture.prompt_maker=false;
    await page.locator('.chat-item[data-chat="123"]').click();
    await page.waitForFunction(() => snapshot?.id==='123');
    assert.equal(await page.locator('#reviewer-model').inputValue(),'saved-reviewer');
    assert.equal(await page.locator('#decisions-enabled').isChecked(),false);
    assert.equal(await page.locator('#learn-procedures').isChecked(),false);
    await page.locator('#settings-button').click();
    await page.locator('#schedule-settings > summary').click();
    await page.locator('#schedule-minutes').fill('60');
    await page.locator('#schedule-create').click();
    await page.waitForFunction(() => document.querySelector('#schedule-status').textContent.startsWith('Scheduled with'));
    assert.equal(scheduleRequests.at(-1).chat_id,'123');
    assert.equal(scheduleRequests.at(-1).interval_seconds,3600);
    assert.deepEqual(errors,[]);
    console.log('Settings: new-chat isolation, submitted workspace, trusted defaults and saved conversation overrides passed.');
  } finally { await browser.close(); }
})().catch(error => {console.error(error);process.exitCode=1;});
