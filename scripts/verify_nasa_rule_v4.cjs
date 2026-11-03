/* Independent host oracle: production runtime, official metadata, real decoder.
 * No publisher and no generated rule implementation. Never uploads anything. */
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),crypto=require('node:crypto'),{spawnSync}=require('node:child_process');
const root=path.resolve(__dirname,'..'),ui=path.resolve(root,'../deepcloud-ui');
const args=process.argv.slice(2),opt=k=>args[args.indexOf(k)+1],sha=b=>crypto.createHash('sha256').update(b).digest('hex');
const {HkDatabase}=require(path.join(ui,'native/runtime/hk-db'));
const engine=require(path.join(ui,'native/runtime/hk-engine'));
const parser=require(path.join(ui,'native/runtime/hiker-rule-parser'));
const fields=['title','version','url','class_name','class_url','find_rule','search_url','searchFind','detail_find_rule'];
let db;const partial={checks:[],observed:{}};
const clean=s=>String(s||'').replace(/<[^>]*>/g,'').trim();
const content=cards=>cards.filter(c=>c?.url&&!['scroll_button','flex_button','input','line','blank_block','rich_text','text_center_1'].includes(c.col_type)&&!/^toast:\/\//.test(c.url)&&!(/^hiker:\/\/empty(?:#|$)/.test(c.url)&&!/@(?:rule|lazyRule)=/.test(c.url)));
async function json(url){const r=await fetch(url,{signal:AbortSignal.timeout(15000)});assert.ok(r.ok,'official source HTTP '+r.status);return r.json();}
async function search(q){return (await json('https://images-api.nasa.gov/search?media_type=video&page_size=100&q='+encodeURIComponent(q))).collection.items;}
function sameTitle(card,data){const a=clean(card.title),b=clean(data?.title);return b.length>3&&(a.includes(b)||b.includes(a)&&a.length>12);}
function matches(cards,reference){return cards.some(c=>reference.some(i=>sameTitle(c,i.data?.[0])));}
function categoryQuery(title){
 const map=[[/火星|mars/i,'mars'],[/地球|earth/i,'earth'],[/空间站|太空站|\biss\b/i,'iss'],[/月球|月亮|moon|lunar/i,'moon'],[/阿尔忒|artemis/i,'artemis'],[/韦伯|webb/i,'webb'],[/阿波罗|apollo/i,'apollo'],[/太阳|solar/i,'solar'],[/哈勃|hubble/i,'hubble']];
 return map.find(([re])=>re.test(clean(title)))?.[1];
}
function categoryMatches(listing,expected){assert.ok(matches(listing,expected),'category label does not match its actual source results');}
function action(url){const m=String(url).match(/@(lazyRule|rule)=/);assert.ok(m,'not an executable rule action');const at=m.index;return {targetUrl:url.slice(0,at),input:url.slice(0,at),actionCode:url.slice(at+m[0].length)};}
async function run(rule,entry,options={}){const result=await engine.runRule({rulePackage:rule,entry,page:1,params:{},...options});assert.equal(result.success,true,entry+': '+(result.error||'runtime failed'));return result;}
function cards(r){assert.ok(Array.isArray(r.data),'production runtime must return cards');return r.data;}
function mediaUrls(value){
 if(typeof value==='string'){try{return mediaUrls(JSON.parse(value));}catch{} if(/^https?:\/\//i.test(value.trim()))return [value.trim().split(';{')[0].split('#')[0]];return [];}
 if(Array.isArray(value))return value.flatMap(mediaUrls);
 if(value&&typeof value==='object')return ['url','urls','data','result','playback'].flatMap(k=>mediaUrls(value[k]));
 return [];
}
async function officialCard(card){
 let id;try{const u=new URL(card.url);id=new URLSearchParams(u.hash.slice(1)).get('id')||u.searchParams.get('nasa_id')||u.searchParams.get('id')||decodeURIComponent(u.pathname.match(/\/(?:video|asset|details)\/([^/]+)/i)?.[1]||'')}catch{}
 const found=id?(await json('https://images-api.nasa.gov/search?media_type=video&nasa_id='+encodeURIComponent(id))).collection.items:await search(clean(card.title));
 return found.find(i=>sameTitle(card,i.data?.[0])&&i.data[0].media_type==='video');
}
async function imported(rule,stage){
 fs.mkdirSync(stage,{recursive:true});
 const normalized=await parser.parseHikerRuleText(parser.exportRuleToStandardCommand(rule));assert.ok(normalized.success&&normalized.isRule,normalized.error);
 db=new HkDatabase();db.init(stage,{seedFoundation:false});assert.equal(db.importRuleRows([normalized.ruleData],'new-target-test','replace').count,1);
 const executable=db.getExecutableRuleByTitle(rule.title);assert.ok(executable);for(const k of ['find_rule','searchFind','detail_find_rule'])if(rule[k]!==undefined)assert.equal(executable[k],rule[k],'import dropped '+k);
 return executable;
}
async function runtimeCheck(rule,config,stage){
 // Post-run diagnostic only: production supports home-script fallback for search.
 // Frozen v3 verdict is preserved; no model receives this implementation.
 for(const k of ['title','find_rule','search_url','detail_find_rule'])assert.ok(rule[k],'missing '+k);
 const text=JSON.stringify(rule);assert.ok(!/127\.0\.0\.1|localhost|child_process|electron_bridge|ctypes|[CD]:[\\/]/i.test(text),'rule depends on local environment');
 const executable=await imported(rule,path.join(stage,'database'));engine.init(stage);
 const home=cards(await run(executable,'home')),first=content(home);assert.ok(first.length,'empty home');
 const second=content(cards(await run(executable,'home',{page:2})));assert.ok(second.length,'empty page two');
 assert.ok(second.some(c=>!first.some(a=>a.url===c.url&&clean(a.title)===clean(c.title))),'page two repeats page one');
 assert.ok(await officialCard(first[0]),'home item is not a real NASA video');assert.ok(await officialCard(second[0]),'page two item is not a real NASA video');
 const checks=['production_imported_rule_runtime','home_official_source','distinct_page_two_official_source'];
 partial.checks=checks;partial.observed.home=first.length;partial.observed.page2=second.length;
 let categories=home.filter(c=>['scroll_button','flex_button'].includes(c.col_type)&&c.url&&!/全部|all|最新|latest|首页/i.test(clean(c.title)));
 let selectedCategories=[];
 for(const c of categories){
  const query=categoryQuery(c.title);if(!query)continue;
  let result;
  if(c.extra?.hikerFilter)result=await run(executable,'home',{params:{[c.extra.kind]:c.extra.value}});
  else if(/@(lazyRule|rule)=/.test(c.url)){result=await run(executable,'action',action(c.url));if(!Array.isArray(result.data))result=await run(executable,'home');}
  else result=await run(executable,'home',{targetUrl:c.url});
  const listing=content(cards(result));assert.ok(listing.length,'empty category '+clean(c.title));assert.ok(await officialCard(listing[0]),'category is not NASA video');
  categoryMatches(listing,await search(query));
  selectedCategories.push({title:clean(c.title),first:clean(listing[0].title)});if(selectedCategories.length===2)break;
 }
 assert.ok(selectedCategories.length>=2,'need at least two working categories');assert.notEqual(selectedCategories[0].first,selectedCategories[1].first,'categories return same first item');checks.push('two_categories_official_source');
 partial.observed.categories=selectedCategories;
 let chosen,reference;
 for(const keyword of ['moon','mars']){
  const found=content(cards(await run(executable,'search',{keyword}))),source=await search(keyword);assert.ok(found.length,'empty search '+keyword);assert.ok(matches(found,source),'search differs from NASA '+keyword);
  checks.push('search:'+keyword);if(!chosen){chosen=found.find(c=>source.some(i=>sameTitle(c,i.data?.[0])));reference=source.find(i=>sameTitle(chosen,i.data?.[0]));}
 }
 const detail=cards(await run(executable,'detail',{targetUrl:chosen.url,item:chosen}));assert.ok(detail.some(c=>sameTitle(c,reference.data[0])||String(c.desc||'').includes(reference.data[0].title)),'detail title missing');
 partial.observed.detail_id=reference.data[0].nasa_id;partial.observed.detail_cards=detail.map(c=>({title:clean(c.title).slice(0,120),type:c.col_type,has_playback:/@lazyRule=|\.mp4/i.test(c.url||'')}));
 const play=detail.filter(c=>c.url&&(/@lazyRule=/.test(c.url)||/\.m(?:p4|3u8)(?:[#?;]|$)/i.test(c.url)));assert.ok(play.length,'no playback action in detail');
 const playback=/@lazyRule=/.test(play[0].url)?await run(executable,'action',action(play[0].url)):play[0].url;
 const urls=mediaUrls(playback),assets=(await json('https://images-api.nasa.gov/asset/'+encodeURIComponent(reference.data[0].nasa_id))).collection.items.map(i=>i.href);
 const media=urls.find(u=>assets.some(a=>{try{return new URL(a).pathname===new URL(u).pathname}catch{return false}}));assert.ok(media,'playback is not from selected NASA asset manifest');checks.push('detail_and_playback_match_selected_official_asset');
 const decoder=spawnSync(config.python,['-X','utf8',path.join(root,'scripts/verify_public_media.py'),'--decoder-dir',config.decoder_dir],{input:JSON.stringify({url:new URL(media).href}),encoding:'utf8',timeout:65000,windowsHide:true});
 let decoded;try{decoded=JSON.parse(decoder.stdout.trim().split(/\r?\n/).at(-1))}catch{throw Error('decoder returned no structured result')}
 assert.ok(decoder.status===0&&decoded.passed,decoded.error||'real decoder failed');checks.push('actual_media_bytes','windowless_decode_over_one_second');
 return {checks,observed:{home:first.length,page2:second.length,categories:selectedCategories,detail_id:reference.data[0].nasa_id},decoder:decoded};
}
async function cloudCheck(rule,workspace,artifact,stage){
 const receipt=JSON.parse(fs.readFileSync(artifact,'utf8')),command=receipt.remote_command;assert.equal(typeof command,'string','publication.json needs remote_command');
 assert.ok(command.length<4096&&/云\d+oooole|https?:\/\//.test(command)&&!/base64:\/\/|localhost|127\.0\.0\.1/.test(command),'not a real short cloud command');
 const sharedText=fs.readFileSync(path.join(workspace,'share-command.txt'),'utf8');
 const baseline=await parser.parseHikerRuleText(parser.exportRuleToStandardCommand(rule)),remote=await parser.parseHikerRuleText(command);assert.ok(remote.success&&remote.isRule,remote.error||'remote rule unavailable');
 const annotated=await parser.parseHikerRuleText(sharedText);assert.ok(annotated.success&&annotated.isRule,'annotated share text does not parse');
 const hashes={};for(const k of fields){assert.deepEqual(remote.ruleData[k],baseline.ruleData[k],'remote field differs: '+k);assert.deepEqual(annotated.ruleData[k],remote.ruleData[k],'share text resolves another rule: '+k);hashes[k]=sha(String(remote.ruleData[k]??''));}
 await imported(remote.ruleData,path.join(stage,'cloud-database'));return {checks:['actual_cloud_command_readback','all_executable_fields_equal','production_private_database_import'],remote_command:command,field_hashes:hashes};
}
async function selfTest(){
 assert.throws(()=>action('hiker://empty'));assert.equal(action('x@lazyRule=.js:1').actionCode,'.js:1');
 assert.ok(matches([{title:'Actual NASA Title'}],[{data:[{title:'Actual NASA Title'}]}]));assert.ok(!matches([{title:'Invented'}],[{data:[{title:'Actual NASA Title'}]}]));
 const invalid=await engine.runRule({rulePackage:{title:'negative fixture',url:'hiker://empty',find_rule:'js:throw Error("intentional-negative")'},entry:'home'});assert.equal(invalid.success,false);
 const valid=await engine.runRule({rulePackage:{title:'positive fixture',url:'hiker://empty',find_rule:'js:setResult([{title:"production-canary",url:"https://example.invalid"}])'},entry:'home'});assert.equal(valid.success,true);assert.equal(valid.data[0].title,'production-canary');
 const temp=fs.mkdtempSync(path.join(require('node:os').tmpdir(),'nasa-fallback-test-'));
 const fallback={title:'native-search-fallback',url:'hiker://empty',search_url:'hiker://empty',find_rule:'js:setResult([{title:"fallback-works",url:"hiker://empty"}]);',detail_find_rule:'js:setResult([])'};
 const executable=await imported(fallback,path.join(temp,'new-directory'));const searched=await run(executable,'search',{keyword:'test'});assert.equal(searched.data[0].title,'fallback-works');
 return {passed:true,target:'https://images.nasa.gov/',checks:['production_canary_and_broken_rule','source_substitution_rejected','action_parse','no_upload']};
}
async function main(){
 const config=JSON.parse(fs.readFileSync(opt('--config'),'utf8')),target='https://images.nasa.gov/';
 const stage=fs.mkdtempSync(path.join(config.stage,'oracle-'));let result;
 if(args.includes('--self-test')){engine.init(stage);result=await selfTest();}
 else{
  const workspace=path.resolve(opt('--workspace')),artifact=path.resolve(opt('--artifact'));assert.ok(artifact.startsWith(workspace+path.sep),'artifact outside workspace');
  const ruleFile=path.join(workspace,'rule.json'),before=sha(fs.readFileSync(ruleFile)),rule=JSON.parse(fs.readFileSync(ruleFile,'utf8').replace(/^\uFEFF/,''));assert.ok(!Array.isArray(rule),'single JSON rule required');
  result=opt('--phase')==='cloud'?await cloudCheck(rule,workspace,artifact,stage):await runtimeCheck(rule,config,stage);
  assert.equal(sha(fs.readFileSync(ruleFile)),before,'rule changed during verification');result={passed:true,target,rule_sha256:before,...result,android_verified:false,gui_used:false};
 }
 return result;
}
async function cleanup(){await engine.terminateAllWorkers().catch(()=>{});db?.db?.close();require(path.join(ui,'native/runtime/hk-db')).db?.close()}
function finish(result,code){process.stdout.write(JSON.stringify(result)+'\n',()=>process.exit(code))}
module.exports={mediaUrls,categoryQuery,categoryMatches};
if(require.main===module)main().then(async result=>{await cleanup();finish(result,0)}).catch(async e=>{await cleanup();finish({passed:false,target:'https://images.nasa.gov/',error:e.message,partial},1)});
