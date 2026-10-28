/* CUE — the live "Cursor" mark and shared helpers, ported from the original CUE design.
   The mark is built from 4 lines and a lamp dot. Each state is a position of these primitives;
   unused lines shrink into an invisible point. Every frame the values are pulled toward the target by a spring.
   The "thinking" dots sit 1 to the right (center 12, like the circle); their jump is unchanged.
   In start and listen the joint box of the bars (with half the stroke) and the lamp is centered on x = 12, so the mark
   sits in the middle of the circle: start for the lamp's full size while live (r 2.3; the static 1.9 sits 0.2 left),
   listen at the middle level (the lamp grows with the level, so the box moves by 0.25 at levels 0 and 1). In listen the
   bars grow from y = 12 and the lamp keeps its place at their top right. idle is not centered: the hover capsule's
   margins in drop.css are measured from where its bar and lamp are painted. */
(function(){
const TAU=Math.PI*2;
const L=(x1,y1,x2,y2,o=1)=>({x1,y1,x2,y2,o});
const PT=(x,y)=>({x1:x,y1:y,x2:x,y2:y,o:0});
const DOT=(x,y)=>({x1:x,y1:y,x2:x,y2:y,o:1});
const BAR=(x,cy,h)=>({x1:x,y1:cy-h/2,x2:x,y2:cy+h/2,o:1});
const ROT=(cx,cy,len,a)=>{const dx=Math.sin(a)*len/2,dy=-Math.cos(a)*len/2;return {x1:cx-dx,y1:cy-dy,x2:cx+dx,y2:cy+dy,o:1}};
function pack(lines,extra){const o={};lines.forEach((l,i)=>{o['a'+i]=l.x1;o['b'+i]=l.y1;o['c'+i]=l.x2;o['d'+i]=l.y2;o['o'+i]=l.o});return Object.assign(o,extra)}
const shake=t=>t<460?Math.sin(t/26)*1.3*(1-t/460):0;

/* Mark states on a 24×24 grid. */
const STATES=['idle','start','listen','think','done','cancel','error','ask','translate'];
function pose(s,c){
 const {t,lvl,jit,now}=c;
 switch(s){
  case 'idle':{const b=c.static?1:(Math.floor(t/560)%2?.12:1),l=L(11,5,11,19,b);return pack([l,{...l},{...l},PT(11,12)],{dx:17.4,dy:6.2,dr:1.9,do:1})}
  case 'start':{const b=c.static?1:(Math.floor(t/140)%2?.4:1),l=L(8.2,5,8.2,19,b);return pack([l,{...l},{...l},PT(8.2,12)],{dx:14.6,dy:6.2,dr:c.static?1.9:Math.min(2.3,.4+t/180),do:1})}
  case 'listen':{const h=i=>3+11.5*lvl*[.72,1,.8][i]*(.5+.5*jit[i]);return pack([BAR(4.8,12,h(0)),BAR(9.3,12,h(1)),BAR(13.8,12,h(2)),PT(9.3,12)],{dx:18.3,dy:4.5,dr:1.5+lvl,do:1})}
  case 'think':{const y=i=>12.5-2.4*Math.max(0,Math.sin(now/150-i*.9));return pack([DOT(7.5,y(0)),DOT(12,y(1)),DOT(16.5,y(2)),PT(12,12)],{dx:12,dy:12.5,dr:0,do:0})}
  case 'done':return pack([L(6,12.5,10,16.5),L(10,16.5,18,8.5),PT(10,16.5),PT(12,12)],{dx:18,dy:8.5,dr:0,do:0});
  case 'error':{const sh=shake(t),l=L(12+sh,5,12+sh,14);return pack([l,{...l},{...l},PT(12,12)],{dx:12+sh,dy:19.2,dr:1.6,do:1})}
  case 'cancel':return pack([L(7,7,17,17),L(17,7,7,17),PT(12,12),PT(12,12)],{dx:12,dy:12,dr:0,do:0});
  case 'ask':{const a=c.static?0:now/1600;return pack([ROT(12,12,14,a),ROT(12,12,14,a+Math.PI/3),ROT(12,12,14,a+2*Math.PI/3),PT(12,12)],{dx:12,dy:12,dr:0,do:0})}
  case 'translate':return pack([L(5,9,19,9),L(19,9,15.5,5.5),L(19,15,5,15),L(5,15,8.5,18.5)],{dx:12,dy:12,dr:0,do:0});
 }
}
const toneOf=s=>s==='error'?'warn':s==='ask'?'ask':'';
const SW=2.2;

class Morph{
 /* g — a <g> inside an svg with viewBox 0 0 24 24 (or transformed into such a grid). */
 constructor(g,o={}){
  this.g=g;g.classList.add('mk');
  g.innerHTML=[0,1,2,3].map(()=>`<line class="m-l" stroke-width="${o.sw||SW}"/>`).join('')+'<circle class="m-dot"/>';
  this.lines=[...g.querySelectorAll('.m-l')];this.dot=g.querySelector('.m-dot');
  this.state=o.state||'idle';this.t0=0;this.k=o.k||.17;this.d=o.d||.7;
  const ctx={t:0,lvl:o.lvl??0,jit:o.jit||[.7,.95,.75,.6],now:0,static:!!o.static};
  this.x=pose(this.state,ctx);this.v={};for(const k in this.x)this.v[k]=0;
  this.tone(o.tone??toneOf(this.state));this.render();
 }
 tone(t){if(t)this.g.dataset.tone=t;else delete this.g.dataset.tone}
 set(s,now,tone){if(s!==this.state){this.state=s;this.t0=now}this.tone(tone??toneOf(s))}
 tick(now,lvl=0,jit=[.5,.5,.5,.5]){
  const tg=pose(this.state,{t:now-this.t0,lvl,jit,now,static:false});
  for(const k in tg){const a=(tg[k]-this.x[k])*this.k;this.v[k]=(this.v[k]+a)*this.d;this.x[k]+=this.v[k]}
  this.render();
 }
 render(){
  const x=this.x;
  this.lines.forEach((l,i)=>{l.setAttribute('x1',x['a'+i].toFixed(3));l.setAttribute('y1',x['b'+i].toFixed(3));l.setAttribute('x2',x['c'+i].toFixed(3));l.setAttribute('y2',x['d'+i].toFixed(3));l.setAttribute('opacity',Math.min(1,Math.max(0,x['o'+i])).toFixed(3))});
  this.dot.setAttribute('cx',x.dx.toFixed(3));this.dot.setAttribute('cy',x.dy.toFixed(3));this.dot.setAttribute('r',Math.max(0,x.dr).toFixed(3));this.dot.setAttribute('opacity',Math.min(1,Math.max(0,x.do)).toFixed(3));
 }
}

/* App icon: a graphite plate (macOS squircle / soft Windows square) + the idle mark. */
const SQ=(()=>{const n=5,c=50,r=41,N=120;let d='';for(let i=0;i<N;i++){const t=i/N*TAU,ct=Math.cos(t),st=Math.sin(t);d+=(i?'L':'M')+(c+r*Math.sign(ct)*Math.abs(ct)**(2/n)).toFixed(2)+' '+(c+r*Math.sign(st)*Math.abs(st)**(2/n)).toFixed(2)}return d+'Z'})();
let UID=0;
function plate(os,size,state='idle'){
 const u='cue'+(UID++),mac=os==='mac',shape=mac?`<path d="${SQ}"/>`:'<rect x="8" y="8" width="84" height="84" rx="17"/>';
 const w=document.createElement('div');
 w.innerHTML=`<svg viewBox="0 0 100 100" width="${size}" height="${size}" aria-hidden="true" style="display:block"><defs><filter id="${u}s" x="-25%" y="-20%" width="150%" height="160%"><feDropShadow dx="0" dy="${mac?2.6:1.8}" stdDeviation="${mac?2.6:2}" flood-color="#000" flood-opacity="${mac?.3:.24}"/></filter><linearGradient id="${u}p" x1="0" y1="0" x2="${mac?0:.5}" y2="1"><stop offset="0" stop-color="#3C3C42"/><stop offset="1" stop-color="#0E0E10"/></linearGradient><linearGradient id="${u}h" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity=".1"/><stop offset=".5" stop-color="#fff" stop-opacity="0"/></linearGradient><clipPath id="${u}c">${shape}</clipPath></defs><g filter="url(#${u}s)" fill="url(#${u}p)">${shape}</g><g clip-path="url(#${u}c)"><g class="art" transform="translate(18.8 18.8) scale(2.6)" style="color:#F4F4F6;--rec:#FF4A1C"></g><rect width="100" height="100" fill="url(#${u}h)"/></g><g fill="none" stroke="rgba(255,255,255,.14)" stroke-width=".9">${shape}</g></svg>`;
 new Morph(w.querySelector('.art'),{state,static:true,tone:''});
 return w.firstElementChild;
}
/* Menu bar / tray icon: monochrome, the lamp is in color while recording. */
function glyph(size=18,o={}){
 const ns='http://www.w3.org/2000/svg',s=document.createElementNS(ns,'svg');
 s.setAttribute('viewBox','0 0 24 24');s.setAttribute('width',size);s.setAttribute('height',size);s.style.display='block';s.style.overflow='visible';
 const g=document.createElementNS(ns,'g');s.appendChild(g);
 const m=new Morph(g,{state:o.state||'idle',static:true,lvl:o.lvl??.7,tone:o.rec?(o.tone||''):'mono'});
 return o.live?{svg:s,morph:m}:s;
}

/* Voice imitation: syllables, pauses, light noise across the bars. An external level (microphone) can be fed in. */
class Voice{
 constructor(){this.lvl=0;this.tgt=0;this.nx=0;this.jit=[.5,.5,.5,.5];this.ext=null}
 step(now,active){
  if(!active)this.tgt=0;
  else if(this.ext!=null)this.tgt=this.ext;
  else if(now>this.nx){if(Math.random()<.16){this.tgt=.05+Math.random()*.06;this.nx=now+200+Math.random()*360}else{this.tgt=.3+Math.random()*.7;this.nx=now+80+Math.random()*160}}
  this.lvl+=(this.tgt-this.lvl)*(active?.1:.12);
  for(let i=0;i<4;i++)this.jit[i]+=(Math.random()-this.jit[i])*.2;
  return this.lvl;
 }
}
/* A label that changes by common letters: the shared beginning stays, the new tail fades in. */
function setLabel(el,txt){
 const old=el.dataset.t||'';let p=0;while(p<old.length&&p<txt.length&&old[p]===txt[p])p++;
 el.dataset.t=txt;el.innerHTML=[...txt].map((ch,i)=>`<span class="${i>=p?'ch-in':''}" style="--i:${i-p}">${ch===' '?'&nbsp;':ch}</span>`).join('');
}
const WALL={
 mac:{light:'radial-gradient(60% 80% at 14% 18%,#FFD7C2 0%,transparent 62%),radial-gradient(55% 70% at 86% 24%,#C5D5FF 0%,transparent 62%),radial-gradient(70% 70% at 55% 102%,#E8D1FF 0%,transparent 66%),linear-gradient(180deg,#F6EEE8,#ECE5F4)',
      dark:'radial-gradient(60% 80% at 14% 14%,#5A2E73 0%,transparent 62%),radial-gradient(55% 70% at 90% 28%,#1F3F80 0%,transparent 62%),radial-gradient(70% 70% at 50% 106%,#7A2E52 0%,transparent 66%),linear-gradient(180deg,#140F22,#0C0B16)'},
 win:{light:'radial-gradient(42% 58% at 68% 64%,rgba(80,140,255,.85) 0%,transparent 70%),radial-gradient(34% 44% at 58% 48%,rgba(175,208,255,.95) 0%,transparent 70%),radial-gradient(60% 80% at 18% 18%,#F2F6FF 0%,transparent 70%),linear-gradient(160deg,#F1F5FC 0%,#C6D9F7 100%)',
      dark:'radial-gradient(42% 58% at 68% 64%,rgba(36,104,226,.78) 0%,transparent 70%),radial-gradient(30% 40% at 60% 50%,rgba(92,160,255,.5) 0%,transparent 70%),radial-gradient(60% 80% at 15% 15%,#0B1A3A 0%,transparent 70%),linear-gradient(160deg,#050B1A 0%,#0A1C42 100%)'}
};
const store={get(k,d){try{const v=localStorage.getItem('cue.'+k);return v==null?d:v}catch(e){return d}},set(k,v){try{localStorage.setItem('cue.'+k,v)}catch(e){}}};

window.CUE={Morph,plate,glyph,Voice,setLabel,WALL,STATES,store,SQ};
})();
