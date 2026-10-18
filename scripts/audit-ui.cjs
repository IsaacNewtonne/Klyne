// Reproducible read-only UI audit. Findings are evidence, never automatic edits.
const fs=require('fs');
const path=require('path');
const {launch}=require('./test-browser.cjs');
async function audit(page,spec){
  const findings=[];
  for(const view of spec.viewports||[{width:1280,height:800},{width:390,height:844}]){
    await page.setViewportSize(view);
    await page.goto(spec.url,{waitUntil:'domcontentloaded',timeout:30000});
    if(spec.ready)await page.locator(spec.ready).waitFor({timeout:15000});
    const result=await page.evaluate(({rules})=>{
      const issues=[];
      if(document.documentElement.scrollWidth>innerWidth+1)issues.push({kind:'horizontal_overflow',actual:document.documentElement.scrollWidth,expected:innerWidth});
      for(const rule of rules||[]){
        const node=document.querySelector(rule.selector);
        if(!node){issues.push({kind:'missing_element',selector:rule.selector});continue;}
        const style=getComputedStyle(node),rect=node.getBoundingClientRect();
        for(const [property,expected] of Object.entries(rule.styles||{}))if(style.getPropertyValue(property).trim()!==expected)issues.push({kind:'style_token',selector:rule.selector,property,expected,actual:style.getPropertyValue(property)});
        if(rule.minWidth&&rect.width<rule.minWidth)issues.push({kind:'width',selector:rule.selector,actual:rect.width,expected:rule.minWidth});
        if(rule.minHeight&&rect.height<rule.minHeight)issues.push({kind:'height',selector:rule.selector,actual:rect.height,expected:rule.minHeight});
        if(rule.text&&!node.textContent.includes(rule.text))issues.push({kind:'text',selector:rule.selector,expected:rule.text});
        if(rule.contrast){
          const rgb=value=>{const m=value.match(/^rgb\((\d+),\s*(\d+),\s*(\d+)\)$/);return m?m.slice(1).map(Number):null;};
          const fg=rgb(style.color),bg=rgb(style.backgroundColor);
          if(!fg||!bg)issues.push({kind:'contrast_inconclusive',selector:rule.selector,reason:'Requires opaque computed foreground/background colors'});
          else {const luminance=c=>c.map(x=>{x/=255;return x<=.04045?x/12.92:((x+.055)/1.055)**2.4;}).reduce((sum,x,i)=>sum+x*[.2126,.7152,.0722][i],0);const a=luminance(fg),b=luminance(bg),ratio=(Math.max(a,b)+.05)/(Math.min(a,b)+.05);if(ratio<rule.contrast)issues.push({kind:'contrast',selector:rule.selector,actual:ratio,expected:rule.contrast});}
        }
      }
      return issues;
    },spec);
    const screenshot=path.join(spec.output,`${view.width}x${view.height}.png`);
    await page.screenshot({path:screenshot,fullPage:true});
    findings.push({viewport:view,screenshot,issues:result});
  }
  return {url:spec.url,checked_at:new Date().toISOString(),passed:findings.every(f=>!f.issues.length),findings,
    scope:'Configured DOM/style/geometry checks and screenshots. No subjective visual-quality certification.'};
}
module.exports={audit};
if(require.main===module)(async()=>{
  const spec=JSON.parse(fs.readFileSync(process.argv[2],'utf8'));
  if(!spec.url||!spec.output)throw Error('Specify url and output in the audit JSON');
  fs.mkdirSync(spec.output,{recursive:true});
  const browser=await launch();try{const result=await audit(await browser.newPage(),spec);fs.writeFileSync(path.join(spec.output,'audit.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result));process.exitCode=result.passed?0:2;}finally{await browser.close();}
})().catch(e=>{console.error(e.message);process.exitCode=1;});
