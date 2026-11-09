/* CUE — the Drop and the tray card. Ported from the original CUE design; the motion is unchanged.
   Differences from the original: the "empty" state (quietly fold into a cross), a close button on errors on hover,
   an error without "retry" (Claude key), the error label from api.errLabel(), all button and label texts from api.text
   (the language is chosen outside this file). Parts only the design's own demo pages used are removed.
   No recording timer: while recording the Drop shows only the mark and stays a circle of the same size.
   Sizes come from drop.css (--h, the Drop's height in every state); nothing here assumes a size.
   The mark is made with {rim:true} (cue.js): its bars centered on the circle, its lamp a status light near the rim.
   Also: the mark, buttons and icons open to their own size measured once (this.nat), not to their scrollWidth;
   with prefers-reduced-motion the mark's spring has almost no inertia (k=20, d=.05) and the tray card folds at once. */
(function(){
const el=h=>{const t=document.createElement('template');t.innerHTML=h.trim();return t.content.firstElementChild};
const SV=d=>`<svg viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${d}</svg>`;
const IC={
 x:SV('<path d="M6 6l8 8M14 6l-8 8"/>'),
 ok:SV('<path d="M4.8 10.4l3.3 3.3 7.1-7.4"/>'),
 undo:SV('<path d="M7.8 4.6 4.4 8l3.4 3.4"/><path d="M4.9 8h7.3a3.6 3.6 0 0 1 0 7.2H9.8"/>'),
 retry:SV('<path d="M15.4 10.2a5.4 5.4 0 1 1-1.7-4"/><path d="M14.3 3.2v3.3H11"/>'),
 net:SV('<path d="M2.8 7.6a10.2 10.2 0 0 1 14.4 0M5.5 10.4a6.4 6.4 0 0 1 9 0"/><path d="M10 14.4h.01" stroke-width="2.6"/><path d="M3.6 3.6l12.8 12.8"/>'),
 mic:SV('<rect x="7.4" y="2.8" width="5.2" height="9" rx="2.6"/><path d="M4.8 9.6a5.2 5.2 0 0 0 10.4 0M10 14.8v2.4"/><path d="M3.6 3.6l12.8 12.8"/>'),
 tr:SV('<path d="M4 7h11.5M12.5 4l3 3-3 3M16 13H4.5M7.5 10l-3 3 3 3"/>'),
 ask:SV('<path d="M10 3.8v12.4M4.6 6.9l10.8 6.2M15.4 6.9 4.6 13.1"/>'),
 book:SV('<path d="M4 4.8A1.8 1.8 0 0 1 5.8 3H16v11.5H5.8A1.8 1.8 0 0 0 4 16.3zM4 16.3A1.8 1.8 0 0 0 5.8 18H16"/><path d="M10 6.2v5M7.5 8.7h5"/>'),
 habit:SV('<path d="M4 16.2c2.2-.2 3.6-1 4.8-2.2l6.6-6.6a1.6 1.6 0 0 0-2.3-2.3L6.5 11.7C5.3 12.9 4.5 14.3 4.3 16"/><path d="M15.6 12.6v3M14.1 14.1h3"/>'),
 copy:SV('<rect x="7" y="7" width="9.5" height="10" rx="2"/><path d="M13 7V5a2 2 0 0 0-2-2H5.5a2 2 0 0 0-2 2v6.5a2 2 0 0 0 2 2H7"/>'),
 key:SV('<rect x="2.8" y="5.5" width="14.4" height="9" rx="2"/><path d="M6 8.6h.01M9 8.6h.01M12 8.6h.01M14.4 8.6h.01M6.8 11.6h6.4"/>'),
 timer:SV('<circle cx="10" cy="11" r="6"/><path d="M10 8v3.2l2 1.3M8 2.8h4"/>'),
 plus:SV('<path d="M10 5v10M5 10h10"/>'),
 model:SV('<path d="M10 2.8l6.2 3.5v7.4L10 17.2l-6.2-3.5V6.3z"/><path d="M3.8 6.3 10 9.8l6.2-3.5M10 9.8v7.4"/>'),
 gear:SV('<circle cx="10" cy="10" r="2.6"/><path d="M10 2.5v2.2M10 15.3v2.2M2.5 10h2.2M15.3 10h2.2M4.7 4.7l1.5 1.5M13.8 13.8l1.5 1.5M4.7 15.3l1.5-1.5M13.8 6.2l1.5-1.5"/>')
};
const RING='<svg class="ring" viewBox="0 0 28 28" width="28" height="28"><circle cx="14" cy="14" r="12.4" pathLength="100"/></svg>';
const RM='(prefers-reduced-motion: reduce)';
/* How long the suggestion's capsule stays open, and its "Undo" after "+" (the rings in drop.css draw the same times) */
const SG_MS={sugg:8000,added:6000};
const reduced=()=>!!(window.matchMedia&&window.matchMedia(RM).matches);
/* parts whose width depends on text: their width is still taken from scrollWidth on every layout */
const TXT=new Set(['chip','lab','act','swd']);
/* While a text part is folded, its scrollWidth misses the right padding of its inner block (.chip>span, .act>span):
   the text overflows that block's zero-width content box. Add it back so the open chip and button are not clipped.
   The other text parts have no such padding (0). */
const padEnd=(k,e)=>TXT.has(k)&&e.firstElementChild?(parseFloat(getComputedStyle(e.firstElementChild).paddingRight)||0):0;

class Drop{
 constructor(host,api,fx=null){
  this.api=api;this.fx=fx;this.live=!fx;
  this.root=el('<div class="pv"><div class="drop" data-s="idle"></div></div>');host.append(this.root);this.p=this.root.firstElementChild;
  this.parts={};this.s='idle';this.hov=!!(fx&&fx.hov);
  const P=(k,h)=>{const e=el(h);e.classList.add('pt');this.parts[k]=e;this.p.append(e);return e};
  const T=api.text;
  P('bx',`<button class="ib soft" title="${T.cancel}" aria-label="${T.cancelName}">${IC.x}</button>`);
  const mk=P('mk','<span class="mkw"><svg viewBox="-8 -8 40 40" width="40" height="40"><g></g></svg></span>');this.morph=new CUE.Morph(mk.querySelector('g'),{rim:true});
  P('mi','<span class="mi"></span>');P('chip','<span class="chip"></span>');P('why','<span class="why"></span>');
  P('lab','<span class="lab"></span>');P('act','<button class="act"></button>');
  P('undo',`<button class="ib solid" title="${T.restore}" aria-label="${T.restore}">${RING}${IC.undo}</button>`);
  P('retry',`<button class="ib solid" title="${T.retry}" aria-label="${T.retry}">${IC.retry}</button>`);
  P('ok',`<button class="ib solid" title="${T.done}" aria-label="${T.done}">${IC.ok}</button>`);
  P('sbk',`<span class="mi sbk">${IC.book}</span>`);P('swd','<span class="lab swd"></span>');
  P('sbt','<button class="ib solid sbt"></button>');
  this.dot=el('<i class="sgdot" aria-hidden="true"></i>');this.root.append(this.dot);
  this.word='';this.missed=false;this.peek=false;this.aw=false;
  /* The suggestion's time: a timer the mouse over the Drop pauses (sgLeft ms left, counted from sgFrom while it runs).
     The ring only draws it, and with reduced motion it stands still. */
  this.sgLeft=0;this.sgFrom=0;this.sgTimer=null;
  /* Natural width of fixed-size parts (the mark, buttons, icons: fractions of --h in drop.css), once, while nothing
     is visible. These parts open to it, never to their scrollWidth: a folded button's sees only the icon inside (the
     button would open as an oval), and the mark's sees its svg, which is as large as the circle and overflows the
     part (the part would open too far, and fold and unfold at another pace). */
  this.nat={};
  for(const [k,e] of Object.entries(this.parts)){if(TXT.has(k)){this.nat[k]=0;continue}e.style.maxWidth='none';this.nat[k]=e.getBoundingClientRect().width;e.style.maxWidth=''}
  const spring=()=>{const r=reduced();this.morph.k=r?20:.17;this.morph.d=r?.05:.7};
  spring();
  if(window.matchMedia){const mq=window.matchMedia(RM);if(mq.addEventListener)mq.addEventListener('change',spring)}
  if(this.live){
   this.parts.bx.addEventListener('click',()=>api.onEsc());this.parts.ok.addEventListener('click',()=>api.onPress());
   for(const k of ['act','undo','retry'])this.parts[k].addEventListener('click',()=>api.onAction());
   this.parts.sbt.addEventListener('click',()=>{const f=api.onSuggest||(()=>{});if(this.s==='sugg'){this.missed=false;this.peek=false;f('add');this.enter('added')}else if(this.s==='added')f('undo')});
   /* a tray card open over the Drop (a habit's) keeps the missed suggestion's capsule closed */
   this.p.addEventListener('mouseenter',()=>{if(this.missed&&this.s==='idle'&&!this.aw){this.peek=true;this.enter('sugg')}});
   this.p.addEventListener('mouseleave',()=>{if(this.peek&&this.s==='sugg'){this.peek=false;this.enter('idle')}});
   this.p.addEventListener('mouseenter',()=>{this.hov=true;this.sgPause();this.layout()});this.p.addEventListener('mouseleave',()=>{this.hov=false;this.sgRun();this.layout()});
  }
  this.enter(fx?fx.state:'idle');
 }
 get mode(){return this.fx?(this.fx.mode||'dictate'):this.api.mode()}
 get err(){return this.fx?(this.fx.err||'net'):this.api.err()}
 enter(s){
  if(s==='start'){this.missed=false;this.peek=false}
  this.s=s;this.p.dataset.s=s;const m=this.mode,now=performance.now();
  if((s==='sugg'&&!this.peek)||s==='added')this.sgStart(SG_MS[s]);else this.sgStop();
  if(s==='sugg'||s==='added'){const T=this.api.text,add=s==='sugg',t=add?T.learnAdd:T.learnUndo;
   this.parts.sbt.innerHTML=RING+(add?IC.plus:IC.undo);this.parts.sbt.title=t;this.parts.sbt.setAttribute('aria-label',t);
   CUE.setLabel(this.parts.swd,this.word)}
  else if(s==='start')this.morph.set(m==='ask'?'ask':m==='translate'?'translate':'start',now);
  else if(s==='listen')this.morph.set('listen',now,m==='ask'?'askrec':'');
  else this.morph.set(s==='empty'?'cancel':s,now);
  this.layout();
 }
 layout(){
  const s=this.s,h=this.hov,m=this.mode,ic=(this.api.lbl?this.api.lbl():'icons')==='icons',p=this.parts,T=this.api.text;
  this.p.classList.toggle('hov',h);
  /* an error label with nothing after it (no retry) would end flush with the capsule: give it its end padding */
  this.p.classList.toggle('lone',s==='error'&&ic&&!(this.api.retry?this.api.retry():true));
  p.mi.innerHTML=m==='translate'?IC.tr:m==='ask'?IC.ask:'';p.mi.classList.toggle('ask',m==='ask');
  p.chip.innerHTML=`<span>${m==='translate'?'EN':T.ask}</span>`;
  p.why.innerHTML=this.err==='mic'?IC.mic:IC.net;
  if(s==='cancel'){CUE.setLabel(p.lab,T.cancelled);p.act.innerHTML=`<span>${T.restore}</span>`}
  if(s==='error'){CUE.setLabel(p.lab,this.api.errLabel?this.api.errLabel():'');p.act.innerHTML=`<span>${T.retry}</span>`}
  const md=m!=='dictate',why=this.err!=='key',rt=this.api.retry?this.api.retry():true;
  const V=ic?{sugg:{sbk:1,swd:1,sbt:1},added:{sbk:1,swd:1,sbt:1},idle:{mk:h},start:{mk:1},listen:{bx:h,mk:1,mi:md,ok:h},think:{mk:1},done:{mk:1},empty:{mk:1},cancel:{mk:1,lab:h,undo:1},error:{bx:h,mk:1,why,lab:h,retry:rt}}
           :{sugg:{sbk:1,swd:1,sbt:1},added:{sbk:1,swd:1,sbt:1},idle:{mk:h},start:{mk:1},listen:{bx:h,mk:1,chip:md,ok:h},think:{mk:1},done:{mk:1},empty:{mk:1},cancel:{mk:1,lab:1,act:1},error:{bx:h,mk:1,lab:1,act:rt}};
  const vis=V[s]||{};
  const open=(k,e)=>TXT.has(k)?Math.max(e.scrollWidth+padEnd(k,e),this.nat[k]||0):this.nat[k]||0;
  for(const [k,e] of Object.entries(p)){const on=!!vis[k];e.classList.toggle('on',on);e.style.maxWidth=on?(open(k,e)+2)+'px':'0px'}
  p.sbk.classList.toggle('ok',s==='added');
  this.p.classList.toggle('sgwait',s==='sugg'&&this.peek);
  this.dot.classList.toggle('on',s==='idle'&&this.missed);
 }
 sgStart(ms){this.sgStop();this.sgLeft=ms;this.sgRun()}
 sgRun(){
  if(!this.live||this.sgLeft<=0||this.sgTimer!=null||this.hov)return;
  this.sgFrom=performance.now();this.sgTimer=setTimeout(()=>{this.sgTimer=null;this.sgLeft=0;this.sgEnd()},this.sgLeft);
 }
 sgPause(){if(this.sgTimer==null)return;clearTimeout(this.sgTimer);this.sgTimer=null;this.sgLeft=Math.max(1,this.sgLeft-(performance.now()-this.sgFrom))}
 sgStop(){if(this.sgTimer!=null)clearTimeout(this.sgTimer);this.sgTimer=null;this.sgLeft=0}
 /* The time ran out: the open capsule leaves a missed suggestion (the dot), the "Undo" one ends it. */
 sgEnd(){const f=this.api.onSuggest||(()=>{});
  if(this.s==='sugg'&&!this.peek){f('expire');this.enter('idle')}else if(this.s==='added'){f('done');this.unoffer()}}
 /* The dictionary suggestion (docs/specs/2026-11-02-corrections.md): open — the capsule with its 8 s ring; missed —
    the dot by the idle bar, and hovering opens the capsule without a ring. */
 offer(word,missed){this.word=word;this.missed=!!missed;this.peek=false;
  if(this.missed){if(this.s==='sugg'||this.s==='added')this.enter('idle');else this.layout()}
  else if(this.s==='idle'||this.s==='sugg')this.enter('sugg');else this.layout()}
 unoffer(){this.word='';this.missed=false;this.peek=false;if(this.s==='sugg'||this.s==='added')this.enter('idle');else this.layout()}
 away(b){this.aw=!!b;this.p.classList.toggle('away',b)}
 size(){return {w:this.p.offsetWidth,h:this.p.offsetHeight}}
 frame(now){const v=this.api.voice;this.morph.tick(now,v.lvl,v.jit)}
}

/* The tray card: grows out of the Drop and folds back into it, starting from and ending in the Drop's own shape
   (radius h/2, whatever that shape is: the circle, a wider capsule, the upright idle strip at a side place).
   Where it is pinned and which way it grows comes from drop.css ([data-place]). */
class Tray{
 constructor(stage,drop,o={}){
  this.drop=drop;this.w=o.width||300;this.isOpen=false;
  this.box=el(`<div class="tray ${o.cls||''}" hidden><div class="in" style="width:${this.w}px"></div></div>`);stage.append(this.box);this.in=this.box.querySelector('.in');
 }
 open(html){
  if(html!=null)this.in.innerHTML=html;
  if(!this.isOpen){const {w,h}=this.drop.size();Object.assign(this.box.style,{width:w+'px',height:h+'px',borderRadius:h/2+'px'});this.box.hidden=false;this.box.classList.remove('open');this.drop.away(true);void this.box.offsetWidth}
  this.isOpen=true;requestAnimationFrame(()=>this.fit());return this.in;
 }
 fit(){if(!this.isOpen)return;Object.assign(this.box.style,{width:this.w+'px',height:this.in.offsetHeight+'px',borderRadius:'20px'});this.box.classList.add('open')}
 width(w){this.w=w;this.in.style.width=w+'px';this.fit()}
 close(){
  if(!this.isOpen)return;this.isOpen=false;const {w,h}=this.drop.size();this.box.classList.remove('open');
  Object.assign(this.box.style,{width:w+'px',height:h+'px',borderRadius:h/2+'px'});
  setTimeout(()=>{if(!this.isOpen){this.box.hidden=true;this.drop.away(false)}},reduced()?0:430);
 }
}

Object.assign(window.CUE,{el,IC,SV,RING,Drop,Tray});
})();
