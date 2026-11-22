const assert = require("node:assert/strict");
const { GitReview } = require("./app.js");
const { skillProposalFiles, mcpProposalLines, proposalRequestLine } = require("./app.js");
// Given a durable proposal, When reviewed, Then only the operator's selected files and edited message are approved.
const gitReview = new GitReview();
const gitVersion={head:"abc",index:"index-a",worktree:"tree-a"};
const gitOverview={workspace_id:2,version:gitVersion,files:[{path:"a.rs",index_status:"M",worktree_status:" "},{path:"b.rs",index_status:" ",worktree_status:"M"}],diff:"+change",checkpoints:[],proposal:{message:"Agent suggestion",paths:["a.rs","b.rs"],expected:gitVersion}};
gitReview.begin(2);
assert.equal(gitReview.receive(2,gitOverview),true);
gitReview.toggle("b.rs",false);
gitReview.message="Operator message";
assert.deepEqual(gitReview.approve(),{type:"commit_selected",message:"Operator message",paths:["a.rs"],expected:gitVersion});
assert.throws(()=>gitReview.toggle("../outside",true));
gitReview.toggle("a.rs",false);
assert.throws(()=>gitReview.approve(),/파일/);
// Given changed files after a proposal, When refreshed, Then approval requires a new review of current changes.
gitReview.receive(2,{...gitOverview,version:{...gitVersion,worktree:"tree-b"}});
assert.equal(gitReview.stale,true);
assert.throws(()=>gitReview.approve(),/변경/);
gitReview.reviewCurrent();
assert.equal(gitReview.stale,false);
gitReview.toggle("a.rs",true);
assert.equal(gitReview.approve().expected.worktree,"tree-b");
// Given two workspaces, When switching or receiving mismatched responses, Then old commit authority is cleared.
gitReview.begin(3);
assert.equal(gitReview.overview,null);
assert.equal(gitReview.message,"");
assert.equal(gitReview.receive(2,gitOverview),false);
assert.equal(gitReview.receive(3,gitOverview),false);
assert.throws(()=>gitReview.approve());
// Given identical trees on another branch, When a proposal is loaded, Then its old branch cannot be approved.
gitReview.begin(2);
gitReview.receive(2,{...gitOverview,version:{...gitVersion,branch:'other'},proposal:{...gitOverview.proposal,expected:{...gitVersion,branch:'feature'}}});
assert.equal(gitReview.stale,true);
const { PluginCatalogState, collectPluginFiles } = require("./app.js");
// Given two workspaces, When a catalog response arrives late, Then it cannot restore old plugin actions.
const pluginState = new PluginCatalogState();
pluginState.begin("workspace-a");
pluginState.receive("workspace-a", {plugins:[
  {id:"user:review",manifest:{name:"review",display_name:"PR Review",description:"Review changes"},enabled:false,skills:[]},
  {id:"repo:docs",manifest:{name:"docs",description:"Write documents"},enabled:true,skills:[]},
],diagnostics:[]});
pluginState.query="PR";
assert.deepEqual(pluginState.visible("all").map(p=>p.id),["user:review"]);
pluginState.query="";
assert.deepEqual(pluginState.visible("repo").map(p=>p.id),["repo:docs"]);
pluginState.selected="repo:docs";
pluginState.begin("workspace-b");
assert.equal(pluginState.receive("workspace-a",{plugins:[],diagnostics:[]}),false);
assert.equal(pluginState.catalog,null);
assert.equal(pluginState.selected,null);
// Given a selected local folder, When imported, Then original binary bytes are uploaded and paths cannot escape it.
const pickedFile=(name,bytes)=>({webkitRelativePath:name,size:bytes.length,arrayBuffer:async()=>Uint8Array.from(bytes).buffer});
async function checkPluginUpload() {
  const request=await collectPluginFiles([
    pickedFile("review/plugin.json",Array.from(Buffer.from('{"name":"review","description":"Review"}'))),
    pickedFile("review/assets/icon.bin",[0,255,128]),
  ],"repo");
  assert.deepEqual(request.files["assets/icon.bin"],[0,255,128]);
  assert.equal(request.scope,"repo");
  await assert.rejects(()=>collectPluginFiles([pickedFile("review/../escape",[1])],"repo"));
  await assert.rejects(()=>collectPluginFiles([pickedFile("review/plugin.json",[1]),pickedFile("other/SKILL.md",[2])],"repo"));
  await assert.rejects(()=>collectPluginFiles([pickedFile("review/plugin.json",[1]),pickedFile("review/PLUGIN.JSON",[2])],"repo"));
  await assert.rejects(()=>collectPluginFiles([pickedFile("review/SKILL.md",[1])],"repo"));
}
checkPluginUpload().catch(error=>{console.error(error);process.exitCode=1;});
// Given the web client, When opening settings or composing, Then management and turn selection are separate.
const html = require("node:fs").readFileSync(require("node:path").join(__dirname,"index.html"),"utf8");
assert.match(html, /id="projectGit"/);
assert.match(html, /id="gitRefresh"/);
assert.match(html, /id="commitProposal"/);
assert.match(html, /id="gitReviewOpen"/);
assert.match(html, /<summary>설정 · 지침<\/summary>/);
assert.match(html, /<summary>설정 · 플러그인<\/summary>/);
const composer = html.slice(html.indexOf('id="attachments"'),html.indexOf('id="prompt"'));
assert.match(composer, /<summary id="skillsSummary">플러그인<\/summary>/);
assert.match(composer, /id="guidanceStatus"/);
assert.doesNotMatch(composer, /생성|편집|새로 고침|guidanceManage/);
const { GuidanceSelection } = require("./app.js");
const { GuidanceEditor } = require("./app.js");
// Given mixed scopes, When changing the management scope, Then files and package skills follow their owner.
const { skillInScope, skillScopeIds } = require("./app.js");
const scopeSkills = [
  {id:"user:skills/common/SKILL.md",enabled:false},
  {id:"repo:.agents/skills/local/SKILL.md",enabled:true},
  {id:"plugin:user:tools/skills/common/SKILL.md",enabled:true},
  {id:"plugin:repo:tools/skills/local/SKILL.md",enabled:true},
];
const scopeFiles = [scopeSkills[0].id,scopeSkills[1].id,"user:skills/broken/SKILL.md","repo:.agents/skills/broken/SKILL.md","repo:AGENTS.md"];
assert.equal(skillInScope(scopeSkills[2].id,"user"),true);
assert.equal(skillInScope(scopeSkills[3].id,"user"),false);
assert.deepEqual(skillScopeIds(scopeSkills,scopeFiles,"user"),[scopeSkills[0].id,scopeSkills[2].id,scopeFiles[2]]);
assert.deepEqual(skillScopeIds(scopeSkills,scopeFiles,"repo"),[scopeSkills[1].id,scopeSkills[3].id,scopeFiles[3]]);
assert.deepEqual(skillScopeIds([],[],"repo"),[]);
const scopedEditor=new GuidanceEditor();
scopedEditor.load({id:scopeSkills[0].id,text:"Original",revision:"v1"});
scopedEditor.text="Draft";
assert.equal(scopedEditor.selectScope("user"),false);
assert.equal(scopedEditor.text,"Draft");
scopedEditor.text="Original";
assert.equal(scopedEditor.selectScope("user"),true);
assert.equal(scopedEditor.file,null);
assert.equal(scopedEditor.destination,"user");
// Given unsaved edits, a workspace switch preserves the draft and stale saves keep their revision.
const editor = new GuidanceEditor();
assert.equal(editor.begin("a"),true);
editor.load({id:"repo:AGENTS.md",text:"Original",revision:"v1"});
editor.text="Edited";
assert.equal(editor.begin("b"),false);
assert.deepEqual(editor.request(),{id:"repo:AGENTS.md",text:"Edited",expected_revision:"v1"});
editor.load({id:"repo:AGENTS.md",text:"Edited",revision:"v2"});
assert.equal(editor.begin("b"),true);
assert.equal(editor.file,null);
// Given a scope switch, a late catalog cannot restore old skill selections.
const guidanceSelection = new GuidanceSelection();
const guidanceCatalog = {skills:[{id:"repo:.agents/skills/test/SKILL.md",enabled:true}]};
guidanceSelection.begin("a");
assert.equal(guidanceSelection.receive("a",guidanceCatalog),true);
guidanceSelection.toggle(guidanceCatalog.skills[0].id);
assert.equal(guidanceSelection.selected.length,1);
guidanceSelection.begin("b");
assert.equal(guidanceSelection.receive("a",guidanceCatalog),false);
assert.equal(guidanceSelection.selected.length,0);
assert.equal(guidanceSelection.receive("b",guidanceCatalog),true);
assert.throws(()=>guidanceSelection.toggle("unknown"));
// Given interleaved skills, When browsing plugins, Then package scopes and standalone skills stay separate.
const groupedCatalog={skills:[
  {id:"plugin:repo:review/skills/check/SKILL.md",enabled:true},
  {id:"repo:.agents/skills/check/SKILL.md",enabled:true},
  {id:"plugin:user:review/skills/check/SKILL.md",enabled:true},
  {id:"plugin:repo:review/skills/verify/SKILL.md",enabled:false},
]};
guidanceSelection.receive("b",groupedCatalog);
assert.deepEqual(guidanceSelection.groups().map(([id,skills])=>[id,skills.map(s=>s.id)]),[
  ["",["repo:.agents/skills/check/SKILL.md"]],
  ["repo:review",["plugin:repo:review/skills/check/SKILL.md","plugin:repo:review/skills/verify/SKILL.md"]],
  ["user:review",["plugin:user:review/skills/check/SKILL.md"]],
]);
guidanceSelection.toggle("plugin:repo:review/skills/check/SKILL.md");
guidanceSelection.toggle("repo:.agents/skills/check/SKILL.md");
assert.deepEqual(guidanceSelection.selected,[{id:"plugin:repo:review/skills/check/SKILL.md"},{id:"repo:.agents/skills/check/SKILL.md"}]);
assert.throws(()=>guidanceSelection.toggle("plugin:repo:review/skills/verify/SKILL.md"));
guidanceSelection.toggle("plugin:repo:review/skills/check/SKILL.md");
assert.deepEqual(guidanceSelection.selected,[{id:"repo:.agents/skills/check/SKILL.md"}]);
const { verificationPassed } = require("./app.js");
// Given host command receipts, When rendering a verified result, Then every command and the exact source version must agree.
const receipt={revision:{head:"abc",fingerprint:"tree-a"},finished_revision:{head:"abc",fingerprint:"tree-a"},checks:[{status:"passed",exit_code:0}]};
assert.equal(verificationPassed(receipt),true);
assert.equal(verificationPassed({...receipt,checks:[{status:"passed",exit_code:1}]}),false);
assert.equal(verificationPassed({...receipt,finished_revision:{head:"abc",fingerprint:"tree-b"}}),false);
assert.equal(verificationPassed({...receipt,finished_revision:null}),false);
const { isWorkEvent } = require("./app.js");
// Given mixed execution and attention events, When routed, Then only execution goes to the work panel.
for (const event of [{type:"verification"}, {type:"tool"}, {type:"input_delivery"}, {type:"runtime_status",status:"retrying"}])
  assert.equal(isWorkEvent(event), true);
for (const event of [{type:"interaction"}, {type:"permission"}, {type:"user_input"}, {type:"elicitation"}, {type:"interaction_resolved"}, {type:"error"}, ...["warning","error","usage_limited"].map(status=>({type:"runtime_status",status}))])
  assert.equal(isWorkEvent(event), false);
const { finishToolCall } = require("./app.js");
// Given a turn has ended, When activity is reopened, Then unfinished tools no longer look active.
assert.equal(finishToolCall({id:"cancelled",status:"in_progress"}).status, "interrupted");
assert.equal(finishToolCall({id:"pending"}).status, "interrupted");
assert.equal(finishToolCall({status:"completed"}).status, "completed");
assert.equal(finishToolCall({status:"failed"}).status, "failed");
const { followsTranscriptTail } = require("./app.js");
// Given a transcript, When output arrives, Then only a reader at its tail follows it.
assert.equal(followsTranscriptTail({scrollHeight:1000,clientHeight:400,scrollTop:600}), true);
assert.equal(followsTranscriptTail({scrollHeight:1000,clientHeight:400,scrollTop:100}), false);
assert.equal(followsTranscriptTail({scrollHeight:200,clientHeight:400,scrollTop:0}), true);
assert.equal(followsTranscriptTail({scrollHeight:1000,clientHeight:400,scrollTop:598.5}), true);
const { restoredTranscriptTop } = require("./app.js");
// Given an older message is being read, When completion reloads history, Then its position remains.
assert.equal(restoredTranscriptTop({scrollHeight:1200,clientHeight:400}, {scrollHeight:1000,clientHeight:400,scrollTop:100}), 100);
assert.equal(restoredTranscriptTop({scrollHeight:1200,clientHeight:400}, {scrollHeight:1000,clientHeight:400,scrollTop:600}), 1200);
assert.equal(restoredTranscriptTop({scrollHeight:1200,clientHeight:400}, null), 1200);
const { questionAnswer } = require("./app.js");
// Given native choices, When answering, Then commas remain part of an option and empty answers cannot be sent.
assert.deepEqual(questionAnswer(true,["A,B","C"],""), ["A,B","C"]);
assert.deepEqual(questionAnswer(false,["B"],""), ["B"]);
assert.deepEqual(questionAnswer(false,["B"],"custom, text"), ["custom, text"]);
assert.deepEqual(questionAnswer(true,["A"],"custom, text"), ["A","custom, text"]);
assert.throws(()=>questionAnswer(false,[],"  "));
assert.throws(()=>questionAnswer(false,["A","B"],""));
const { parseJson } = require("./app.js");
const { mergeToolCall } = require("./app.js");
const tool = mergeToolCall({}, {id:"mcp",status:"in_progress",activity:[{type:"progress",message:"자료 검색 중"}]});
mergeToolCall(tool, {activity:[{type:"progress",message:"자료 검증 중"}]});
mergeToolCall(tool, {status:"completed",output:{text:"Final result"}});
assert.deepEqual(tool.activity, [{type:"progress",message:"자료 검색 중"},{type:"progress",message:"자료 검증 중"}]);
assert.deepEqual(tool.output, {text:"Final result"});
const terminal = mergeToolCall({}, {output_delta:"old stdout",activity:[{type:"terminal_input",process_id:"42",stdin:"계속\n"}]});
mergeToolCall(terminal, {output:"actual stdout",input:[{path:"before.rs"}]});
mergeToolCall(terminal, {input:[]});
assert.equal(terminal.activity[0].stdin, "계속\n");
assert.equal(terminal.output, "actual stdout");
assert.deepEqual(terminal.input, []);
const { codexExecutionPolicy }=require("./app.js");
assert.deepEqual(codexExecutionPolicy("codex","on-request","workspace-write"),{approval_policy:"on-request",sandbox:"workspace-write"});
assert.equal(codexExecutionPolicy("codex","untrusted","read-only"),undefined);
assert.equal(codexExecutionPolicy("claude","never","danger-full-access"),undefined);
assert.throws(()=>codexExecutionPolicy("codex","always","workspace-write"));
const data = parseJson(
  '{"operation_id":18446744073709551615,"request_id":9007199254740993,"text":"a\\\"12345678901234567890 b","sequence":12}',
);
assert.equal(data.operation_id, "18446744073709551615");
assert.equal(data.request_id, "9007199254740993");
assert.equal(data.sequence, 12);
assert.equal(data.text, 'a"12345678901234567890 b');
assert.equal(parseJson('{"n":-9007199254740994}').n, "-9007199254740994");
assert.deepEqual(parseJson("[1,1.25,-2,1e3,null,true]"), [
  1,
  1.25,
  -2,
  1000,
  null,
  true,
]);
console.log("ok: lossless stream IDs, escaped strings and ordinary JSON");
// Given a proposal, When rendered in the project panel, Then review shows scoped context and requires explicit approval.
const {renderGitReview}=require('./app.js');
const element=(tag)=>({tag,children:[],textContent:'',append(...nodes){this.children.push(...nodes);},replaceChildren(...nodes){this.children=nodes;}});
const gitContainer=element('div');gitContainer.ownerDocument={createElement:element};
const flatten=node=>[node,...node.children.flatMap(flatten)];
gitReview.begin(2);gitReview.receive(2,gitOverview);
let approvedGit;
renderGitReview(gitContainer,gitReview,{name:'Project',branch:'feature',path:'/repo/worktree'},false,command=>{approvedGit=command;},()=>{});
assert.match(flatten(gitContainer).map(node=>node.textContent).join(' '),/Project.*feature.*\/repo\/worktree/);
assert.equal(approvedGit,undefined,'Rendering a proposal must never execute it');
const fileCheckbox=flatten(gitContainer).find(node=>node.type==='checkbox' && node.value==='b.rs');
fileCheckbox.checked=false;fileCheckbox.onchange();
const messageField=flatten(gitContainer).find(node=>node.tag==='textarea');
messageField.value='Approved edit';messageField.oninput();
flatten(gitContainer).find(node=>node.textContent==='선택한 파일 커밋 승인').onclick();
assert.deepEqual(approvedGit,{type:'commit_selected',message:'Approved edit',paths:['a.rs'],expected:gitVersion});
renderGitReview(gitContainer,gitReview,{name:'Project'},true,()=>{throw new Error('Busy Git action');},()=>{});
assert.equal(flatten(gitContainer).find(node=>node.textContent==='선택한 파일 커밋 승인').disabled,true);
flatten(gitContainer).find(node=>node.textContent==='선택한 파일 커밋 승인').onclick();
gitReview.receive(2,{...gitOverview,version:{...gitVersion,index:'index-b'}});
renderGitReview(gitContainer,gitReview,{name:'Project'},false,()=>{},()=>{});
assert.equal(flatten(gitContainer).find(node=>node.textContent==='선택한 파일 커밋 승인').disabled,true);
flatten(gitContainer).find(node=>node.textContent==='현재 변경으로 다시 검토').onclick();
assert.equal(gitReview.stale,false);
// Given no proposal, When the operator selects a changed file, Then manual commit uses the same reviewed version.
gitReview.receive(2,{...gitOverview,proposal:null});
gitReview.toggle('b.rs',true);gitReview.message='Manual commit';
assert.deepEqual(gitReview.approve().paths,['b.rs']);
assert.equal(gitReview.approve().expected,gitVersion);
// Given an active agent, When inspecting a changed file, Then reads stay enabled and both diff layers are visible.
let inspectedPath;
renderGitReview(gitContainer,gitReview,{name:'Project',branch:'stale-catalog'},true,()=>{throw new Error('Mutation while busy');},()=>{},path=>{inspectedPath=path;});
const inspectButtons=flatten(gitContainer).filter(node=>node.textContent==='변경 검토');
assert.equal(inspectButtons.length,2);
assert.equal(inspectButtons[1].disabled,false);
inspectButtons[1].onclick();assert.equal(inspectedPath,'b.rs');
const {loadGitDiff}=require('./app.js');
async function checkGitDiffRead() {
  const review=new GitReview();review.begin(2);review.receive(2,{...gitOverview,version:{...gitVersion,branch:'live-branch'}});
  let release,readPath,activeBusy=true,activeJob=71,activeController={stream:true};
  const originalController=activeController;
  const read=loadGitDiff(review,'b.rs',path=>{readPath=path;return new Promise(resolve=>{release=resolve;});},()=>{});
  assert.equal(readPath,'/v1/workspaces/2/diff?path=b.rs');
  assert.equal(review.diffLoading,true);
  release({workspace_id:2,path:'b.rs',staged:'+staged content',unstaged:'+unstaged content',untracked:true,truncated:true});
  await read;
  renderGitReview(gitContainer,review,{name:'Project',branch:'stale-catalog'},activeBusy,()=>{},()=>{});
  const text=flatten(gitContainer).map(node=>node.textContent).join(' ');
  assert.match(text,/live-branch/);assert.doesNotMatch(text,/stale-catalog/);
  assert.match(text,/b.rs.*staged content.*unstaged content/);
  assert.match(text,/미추적/);assert.match(text,/일부만/);
  assert.equal(activeBusy,true);assert.equal(activeJob,71);assert.equal(activeController,originalController);
  const late=loadGitDiff(review,'a.rs',()=>new Promise(resolve=>{release=resolve;}),()=>{});
  review.begin(3);
  release({workspace_id:2,path:'a.rs',staged:'OLD',unstaged:'OLD'});await late;
  assert.equal(review.fileDiff,null);
  review.begin(2);review.receive(2,gitOverview);
  await loadGitDiff(review,'a.rs',async()=>({workspace_id:3,path:'a.rs',staged:'WRONG',unstaged:'WRONG'}),()=>{});
  assert.equal(review.fileDiff,null);assert.match(review.diffError,/작업공간/);
  const older=loadGitDiff(review,'a.rs',()=>new Promise(resolve=>{release=resolve;}),()=>{});
  await loadGitDiff(review,'b.rs',async()=>({workspace_id:2,path:'b.rs',staged:'',unstaged:'NEW'}),()=>{});
  release({workspace_id:2,path:'a.rs',staged:'OLD',unstaged:'OLD'});await older;
  assert.equal(review.fileDiff.path,'b.rs');
  const oldSnapshot=loadGitDiff(review,'a.rs',()=>new Promise(resolve=>{release=resolve;}),()=>{});
  review.receive(2,{...gitOverview,version:{...gitVersion,worktree:'next'}});
  release({workspace_id:2,path:'a.rs',staged:'OLD SNAPSHOT',unstaged:''});await oldSnapshot;
  assert.equal(review.fileDiff,null);
  await loadGitDiff(review,'a.rs',async()=>{throw new Error('Diff unavailable');},()=>{});
  assert.equal(review.diffLoading,false);assert.match(review.diffError,/Diff unavailable/);
  review.receive(2,{...gitOverview,version:{...gitVersion,branch:null}});
  renderGitReview(gitContainer,review,{name:'Project',branch:'stale-catalog'},false,()=>{},()=>{});
  assert.match(flatten(gitContainer).map(node=>node.textContent).join(' '),/분리된 HEAD/);
}
checkGitDiffRead().catch(error=>{console.error(error);process.exitCode=1;});
// Given an agent skill proposal, When shown for approval, Then the operator reviews every file that would be written, manifest first.
const proposal={id:"sp1",skill_id:"repo:.agents/skills/review/SKILL.md",name:"review",content:"---\ndescription: Review\n---\nBody",resources:{"references/b.md":"B","assets/a.txt":"A"},status:"pending",expires_at:1};
assert.deepEqual(skillProposalFiles(proposal),[
  {path:".agents/skills/review/SKILL.md",text:proposal.content},
  {path:".agents/skills/review/assets/a.txt",text:"A"},
  {path:".agents/skills/review/references/b.md",text:"B"},
]);
assert.deepEqual(skillProposalFiles({...proposal,resources:{}}).map(f=>f.path),[".agents/skills/review/SKILL.md"]);
// Given an agent MCP proposal, When shown for approval, Then the operator sees the exact command or endpoint, environment references and rationale.
const mcpBase={id:"mp1",plugin_id:"repo:mcp-fixture-tools",rationale:"Inspect fixtures",status:"pending",expires_at:1};
assert.deepEqual(mcpProposalLines({...mcpBase,server:{name:"fixture_tools",command:"/usr/bin/tool",args:["--serve","a b"]}}),[
  "이름: fixture_tools","실행 파일: /usr/bin/tool","인자: --serve a b","설치 위치: repo:mcp-fixture-tools","이유: Inspect fixtures"]);
assert.deepEqual(mcpProposalLines({...mcpBase,server:{name:"remote",url:"https://example.com/mcp",bearer_token_env_var:"TOKEN",env_http_headers:{"X-Key":"KEY_VAR"}}}),[
  "이름: remote","주소: https://example.com/mcp","Bearer 환경 변수: TOKEN","헤더 환경 변수: X-Key=KEY_VAR","설치 위치: repo:mcp-fixture-tools","이유: Inspect fixtures"]);
// Given the host's environment state, When listed, Then readiness is shown by variable name and never by value.
assert.deepEqual(mcpProposalLines({...mcpBase,server:{name:"remote",url:"https://example.com/mcp",bearer_token_env_var:"TOKEN"},environment:[{variable:"TOKEN",ready:true},{variable:"KEY_VAR",ready:false}]}).slice(3,5),["환경 변수 준비됨: TOKEN","환경 변수 미설정: KEY_VAR"]);
// Given the host-recorded user request, When a proposal is shown, Then the operator can compare it with what was asked; absent context is stated, not hidden.
assert.equal(proposalRequestLine({requested_during:"Please review my change"}),"대화의 마지막 요청: Please review my change");
assert.equal(proposalRequestLine({}),"대화의 마지막 요청: (확인할 수 없음 — 에이전트가 스스로 제안했을 수 있습니다)");
