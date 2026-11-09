/* CUE — the live "Cursor" mark and shared helpers, ported from the original CUE design.
   The mark is built from 5 lines and a lamp dot. Each state is a position of these primitives;
   unused lines shrink into an invisible point. Every frame the values are pulled toward the target by a spring.
   The "thinking" dots sit 1 to the right (center 12, like the circle); their jump is unchanged. They are the voice
   bars' own lines (0, 2, 4) on the bars' own x, so going from listen into think each bar shrinks into its dot where it
   stands; done draws its check from the same lines (the first dot the short stroke, the middle one the long).
   In start and listen the bars are centered in the circle: the three voice bars on 7.5 / 12 / 16.5 growing from y = 12,
   so they stay centered at every level. The lamp is an accent at their top right, not part of the centering: +9 from
   the middle bar, 7.5 above the bars' center line (21, 4.5). The mark is kept to the 24 grid. Every still pose lies
   0.5 or more inside it (the listen lamp at its largest, r 2.5, reaches 23.5). Between poses the spring
   overshoots: the hidden lamp of ask and translate is parked at the listen lamp's spot, so their way into listen stays
   inside, and wherever the lamp flies from (the
   error lamp, or the hidden one of think and cancel, both near the middle, into listen through a short start) it is
   drawn no closer than 0.5 to the grid's edge, a line no still pose crosses. A test runs every way into listen.
   idle has the same bars and lamp.
   The Drop's mark keeps its lines to the grid too. In the Drop (a Morph made with {rim:true}) the bars stay centered on the circle and the lamp is a status light near
   the circle's rim instead: at RIM_LAMP, up and right of the middle at 45 degrees (see below). The circle is the 24 grid
   with 8 more on every side (drop.css: the mark is 0.6 of the Drop's height), so its rim is 20 from the middle and the
   lamp lies partly outside the grid; the Drop lets the mark's part paint as far as the circle. There every pose parks
   the hidden lamp at its spot, so recording ends with the light going out where it is, and the lamp is drawn no closer
   than RIM_GAP to the rim, a line no still pose crosses. */
(function(){
const TAU=Math.PI*2;
const L=(x1,y1,x2,y2,o=1)=>({x1,y1,x2,y2,o});
const PT=(x,y)=>({x1:x,y1:y,x2:x,y2:y,o:0});
const DOT=(x,y)=>({x1:x,y1:y,x2:x,y2:y,o:1});
const BAR=(x,cy,h)=>({x1:x,y1:cy-h/2,x2:x,y2:cy+h/2,o:1});
const ROT=(cx,cy,len,a)=>{const dx=Math.sin(a)*len/2,dy=-Math.cos(a)*len/2;return {x1:cx-dx,y1:cy-dy,x2:cx+dx,y2:cy+dy,o:1}};
function pack(lines,extra){const o={};lines.forEach((l,i)=>{o['a'+i]=l.x1;o['b'+i]=l.y1;o['c'+i]=l.x2;o['d'+i]=l.y2;o['o'+i]=l.o});return Object.assign(o,extra)}
const shake=t=>t<460?Math.sin(t/26)*1.3*(1-t/460):0;

/* One mark from rest to the end of a dictation: three voice bars on x 7.5 / 12 / 16.5 (lines 0, 2, 4; the other two
   are parked between them) and the lamp at their top right (21, 4.5). Only the bars' heights change.
   idle is the dictation wave: the bars as tall as a loud voice, 16 at the middle in the voice's proportions, roll as a
   slow wave (2.4 s, each 0.5 s after the one before) and the lamp, r 1.9, fades in and out (1.8 s); its place and size
   stay, so the hover capsule's margins (drop.css) hold.
   start is listen before the microphone has sent anything: the bars at rest while the lamp grows red. Listening then
   changes nothing: the bars move only with the voice. */
const VOICE_X=[7.5,12,16.5],VOICE_K=[.72,1,.8],IDLE_H=VOICE_K.map(k=>16*k);
const voiceBars=h=>[BAR(VOICE_X[0],12,h(0)),PT(9.75,12),BAR(VOICE_X[1],12,h(1)),PT(14.25,12),BAR(VOICE_X[2],12,h(2))];
/* The Drop's lamp: 45 degrees up and right of the circle's middle, 10 * sqrt(2) (14.1) from it. Its edge stays 3.4
   from the rim at its largest (r 2.5) and 4.4 in start (r 1.5), and 3.6 from the tallest bar's cap at idle (r 1.9).
   RIM is the circle's radius on the grid, RIM_GAP the closest the lamp is drawn to it (overshoots only). */
const RIM_LAMP={x:22,y:2},RIM=20,RIM_GAP=3;
/* Mark states on a 24×24 grid. */
const STATES=['idle','start','listen','think','done','cancel','error','ask','translate'];
function pose(s,c){
 const {t,lvl,jit,now}=c,rim=!!c.rim,lx=rim?RIM_LAMP.x:21,ly=rim?RIM_LAMP.y:4.5;
 /* a hidden lamp: in the Drop it goes out at its own spot; elsewhere where the pose puts it */
 const off=(x,y)=>rim?{dx:lx,dy:ly,dr:0,do:0}:{dx:x,dy:y,dr:0,do:0};
 switch(s){
  case 'idle':{const s=i=>c.static?1:.725+.275*Math.sin(TAU*(t+i*500)/2400);return pack(voiceBars(i=>IDLE_H[i]*s(i)),{dx:lx,dy:ly,dr:1.9,do:c.static?1:.775+.225*Math.sin(TAU*t/1800)})}
  case 'start':return pack(voiceBars(()=>3),{dx:lx,dy:ly,dr:c.static?1.5:Math.min(1.5,.4+t/180),do:1});
  case 'listen':{const h=i=>3+11.5*lvl*VOICE_K[i]*(.5+.5*jit[i]);return pack(voiceBars(h),{dx:lx,dy:ly,dr:1.5+lvl,do:1})}
  case 'think':{const y=i=>12.5-2.4*Math.max(0,Math.sin(now/150-i*.9));return pack([DOT(7.5,y(0)),PT(9.75,12),DOT(12,y(1)),PT(14.25,12),DOT(16.5,y(2))],off(12,12.5))}
  case 'done':return pack([L(6,12.5,10,16.5),PT(10,16.5),L(10,16.5,18,8.5),PT(12,12),PT(10,16.5)],off(18,8.5));
  case 'error':{const sh=shake(t),l=L(12+sh,5,12+sh,14);return pack([l,{...l},{...l},PT(12,12),PT(12,12)],{dx:12+sh,dy:19.2,dr:1.6,do:1})}
  case 'cancel':return pack([L(7,7,17,17),L(17,7,7,17),PT(12,12),PT(12,12),PT(12,12)],off(12,12));
  case 'ask':{const a=c.static?0:now/1600;return pack([ROT(12,12,14,a),ROT(12,12,14,a+Math.PI/3),ROT(12,12,14,a+2*Math.PI/3),PT(12,12),PT(12,12)],{dx:lx,dy:ly,dr:0,do:0})}
  case 'translate':return pack([L(5,9,19,9),L(19,9,15.5,5.5),L(19,15,5,15),L(5,15,8.5,18.5),PT(12,12)],{dx:lx,dy:ly,dr:0,do:0});
 }
}
const toneOf=s=>s==='error'?'warn':s==='ask'?'ask':'';
const SW=2.2;

class Morph{
 /* g — a <g> inside an svg with viewBox 0 0 24 24 (or transformed into such a grid). */
 constructor(g,o={}){
  this.g=g;g.classList.add('mk');
  g.innerHTML=[0,1,2,3,4].map(()=>`<line class="m-l" stroke-width="${o.sw||SW}"/>`).join('')+'<circle class="m-dot"/>';
  this.lines=[...g.querySelectorAll('.m-l')];this.dot=g.querySelector('.m-dot');
  this.state=o.state||'idle';this.t0=0;this.k=o.k||.17;this.d=o.d||.7;this.rim=!!o.rim;
  const ctx={t:0,lvl:o.lvl??0,jit:o.jit||[.7,.95,.75,.6],now:0,static:!!o.static,rim:this.rim};
  this.x=pose(this.state,ctx);this.v={};for(const k in this.x)this.v[k]=0;
  this.tone(o.tone??toneOf(this.state));this.render();
 }
 tone(t){if(t)this.g.dataset.tone=t;else delete this.g.dataset.tone}
 set(s,now,tone){if(s!==this.state){this.state=s;this.t0=now}this.tone(tone??toneOf(s))}
 tick(now,lvl=0,jit=[.5,.5,.5,.5]){
  const tg=pose(this.state,{t:now-this.t0,lvl,jit,now,static:false,rim:this.rim});
  for(const k in tg){const a=(tg[k]-this.x[k])*this.k;this.v[k]=(this.v[k]+a)*this.d;this.x[k]+=this.v[k]}
  this.render();
 }
 render(){
  const x=this.x;
  this.lines.forEach((l,i)=>{l.setAttribute('x1',x['a'+i].toFixed(3));l.setAttribute('y1',x['b'+i].toFixed(3));l.setAttribute('x2',x['c'+i].toFixed(3));l.setAttribute('y2',x['d'+i].toFixed(3));l.setAttribute('opacity',Math.min(1,Math.max(0,x['o'+i])).toFixed(3))});
  const r=Math.max(0,x.dr);let cx,cy;
  if(this.rim){const ox=x.dx-12,oy=x.dy-12,d=Math.hypot(ox,oy),m=RIM-RIM_GAP-r,k=d>m?m/d:1;cx=12+ox*k;cy=12+oy*k}
  else{const inside=v=>Math.min(Math.max(v,r+.5),24-r-.5);cx=inside(x.dx);cy=inside(x.dy)}
  this.dot.setAttribute('cx',cx.toFixed(3));this.dot.setAttribute('cy',cy.toFixed(3));this.dot.setAttribute('r',Math.max(0,x.dr).toFixed(3));this.dot.setAttribute('opacity',Math.min(1,Math.max(0,x.do)).toFixed(3));
 }
}

/* App icon: a graphite plate (macOS squircle / soft Windows square) + the idle mark, what it paints centered on the
   plate. */
const SQ=(()=>{const n=5,c=50,r=41,N=120;let d='';for(let i=0;i<N;i++){const t=i/N*TAU,ct=Math.cos(t),st=Math.sin(t);d+=(i?'L':'M')+(c+r*Math.sign(ct)*Math.abs(ct)**(2/n)).toFixed(2)+' '+(c+r*Math.sign(st)*Math.abs(st)**(2/n)).toFixed(2)}return d+'Z'})();
let UID=0;
function plate(os,size,state='idle'){
 const u='cue'+(UID++),mac=os==='mac',shape=mac?`<path d="${SQ}"/>`:'<rect x="8" y="8" width="84" height="84" rx="17"/>';
 const w=document.createElement('div');
 w.innerHTML=`<svg viewBox="0 0 100 100" width="${size}" height="${size}" aria-hidden="true" style="display:block"><defs><filter id="${u}s" x="-25%" y="-20%" width="150%" height="160%"><feDropShadow dx="0" dy="${mac?2.6:1.8}" stdDeviation="${mac?2.6:2}" flood-color="#000" flood-opacity="${mac?.3:.24}"/></filter><linearGradient id="${u}p" x1="0" y1="0" x2="${mac?0:.5}" y2="1"><stop offset="0" stop-color="#3C3C42"/><stop offset="1" stop-color="#0E0E10"/></linearGradient><linearGradient id="${u}h" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity=".1"/><stop offset=".5" stop-color="#fff" stop-opacity="0"/></linearGradient><clipPath id="${u}c">${shape}</clipPath></defs><g filter="url(#${u}s)" fill="url(#${u}p)">${shape}</g><g clip-path="url(#${u}c)"><g class="art" transform="translate(11.9 19.2) scale(2.6)" style="color:#F4F4F6;--rec:#FF4A1C"></g><rect width="100" height="100" fill="url(#${u}h)"/></g><g fill="none" stroke="rgba(255,255,255,.14)" stroke-width=".9">${shape}</g></svg>`;
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

window.CUE={Morph,plate,glyph,Voice,setLabel,WALL,STATES,store,SQ,RIM_LAMP,RIM,RIM_GAP};
})();
