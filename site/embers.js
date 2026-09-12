// Ambient embers: one lightweight canvas behind the interface.
(() => {
  const canvas = document.createElement('canvas');
  canvas.id = 'ambient-embers'; canvas.setAttribute('aria-hidden', 'true');
  document.body.prepend(canvas);
  const ctx = canvas.getContext('2d'); if (!ctx) { canvas.remove(); return; }
  const motion = matchMedia('(prefers-reduced-motion: reduce)');
  let width = 0, height = 0, raf = 0, previous = 0;
  const particles = [];
  let cpuFraction = 0, displayedLoad = 0, particleBudget = 60, baselineEmbers = 24;
  const pointer = {x:0, y:0, active:false, vx:0, vy:0, time:0};
  addEventListener('pointermove', event => {
    if(event.pointerType === 'touch' || motion.matches) return;
    const now = performance.now(), elapsed = Math.max(16, now-pointer.time)/1000;
    pointer.vx = pointer.active ? Math.max(-700,Math.min(700,(event.clientX-pointer.x)/elapsed)) : 0;
    pointer.vy = pointer.active ? Math.max(-700,Math.min(700,(event.clientY-pointer.y)/elapsed)) : 0;
    pointer.x=event.clientX; pointer.y=event.clientY; pointer.time=now; pointer.active=true;
  }, {passive:true});
  function releasePointer() { pointer.active=false; pointer.vx=pointer.vy=0; }
  document.documentElement.addEventListener('pointerleave',releasePointer);
  addEventListener('blur',releasePointer);
  function spawn(initial = false) {
    return {x:Math.random()*width,y:initial?Math.random()*height:height+12,
      speed:12+Math.random()*34,drift:(Math.random()-.5)*18,
      radius:.8+Math.random()*1.6,alpha:.35+Math.random()*.4,
      phase:Math.random()*Math.PI*2,frequency:.25+Math.random()*.65,
      life:0,turn:Math.random()*6,red:Math.random()>.25,vx:0,vy:0,visibility:0};
  }
  function resize() {
    width = innerWidth; height = innerHeight;
    const dpr = Math.min(devicePixelRatio || 1, 1.5);
    canvas.width = Math.round(width*dpr); canvas.height = Math.round(height*dpr);
    ctx.setTransform(dpr,0,0,dpr,0,0);
    particles.length = 0;
    particleBudget=Math.min(150,Math.max(50,Math.round(width*height/12000)));
    baselineEmbers=Math.min(42,Math.max(18,Math.round(width*height/28000)));
    for(let i=0;i<particleBudget;i++) { const p=spawn(true);p.visibility=i<baselineEmbers?1:0;particles.push(p); }
    draw(0);
  }
  function draw(dt) {
    ctx.clearRect(0,0,width,height);
    const working=submitting || ['Planning','Working','Reviewing','Upgrading'].includes(snapshot?.status);
    const targetLoad=working?Math.max(.85,cpuFraction):cpuFraction;
    displayedLoad+=(targetLoad-displayedLoad)*(1-Math.exp(-dt*1.5));
    const targetCount=baselineEmbers+(particleBudget-baselineEmbers)*displayedLoad;
    for(let i=0;i<particles.length;i++) {
      particles[i].visibility+=((i<targetCount?1:0)-particles[i].visibility)*(1-Math.exp(-dt*2));
      if(particles[i].visibility<.005) continue;
      let p=particles[i]; p.life+=dt;
      let proximity=0;
      if(pointer.active && !motion.matches) {
        const dx=p.x-pointer.x, dy=p.y-pointer.y, distance=Math.hypot(dx,dy);
        proximity=Math.max(0,1-distance/190);
        if(proximity>0) {
          const nx=dx/Math.max(1,distance), ny=dy/Math.max(1,distance), force=proximity*proximity;
          const wake=Math.exp(-Math.max(0,performance.now()-pointer.time)/180);
          p.vx+=(nx*170-ny*115+pointer.vx*wake*.65)*force*dt;
          p.vy+=(ny*170+nx*115+pointer.vy*wake*.65)*force*dt;
        }
      }
      const damping=Math.exp(-2.2*dt);
      p.vx=Math.max(-160,Math.min(160,p.vx*damping));
      p.vy=Math.max(-160,Math.min(160,p.vy*damping));
      p.y+=(-p.speed+p.vy)*dt;
      p.x+=(p.drift+Math.sin(p.phase+p.life*p.frequency)*12+p.vx)*dt;
      if(p.y < -20 || p.x < -30 || p.x > width+30) { const visibility=p.visibility;p=particles[i]=spawn();p.visibility=visibility; }
      const fade=Math.min(1,Math.max(0,p.y/90));
      const alpha=Math.min(.85,p.alpha*fade*(.76+.24*Math.sin(p.phase+p.life*1.8))*(1+proximity*.65));
      ctx.save(); ctx.translate(p.x,p.y); ctx.rotate(p.turn+p.life*.3);
      ctx.fillStyle=p.red?'#ff392b':'#ff6b35';
      ctx.globalAlpha=alpha*p.visibility*.13;
      ctx.beginPath();ctx.arc(0,0,p.radius*4,0,Math.PI*2);ctx.fill();
      ctx.globalAlpha=alpha*p.visibility;
      ctx.fillRect(-p.radius/2,-p.radius,p.radius,p.radius*2.4);
      ctx.restore();
    }
  }
  function tick(time) {
    if(time-previous>=32) {
      draw(previous?Math.min((time-previous)/1000,.06):0);
      previous=time;
    }
    raf=requestAnimationFrame(tick);
  }
  function sync() {
    cancelAnimationFrame(raf); previous=0;
    releasePointer();
    if(!document.hidden && !motion.matches) raf=requestAnimationFrame(tick);
    else draw(0);
  }
  addEventListener('resize',resize);
  document.addEventListener('visibilitychange',sync);
  motion.addEventListener('change',sync);
  resize(); sync();
})();
