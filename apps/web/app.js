"use strict";
// Keep u64 operation/interaction IDs exact without changing text inside JSON strings.
function parseJson(source) {
  let quoted = false,
    escaped = false,
    out = "";
  for (let i = 0; i < source.length;) {
    const c = source[i];
    if (quoted) {
      out += c;
      i++;
      if (escaped) escaped = false;
      else if (c === "\\") escaped = true;
      else if (c === '"') quoted = false;
    } else if (c === '"') {
      quoted = true;
      out += c;
      i++;
    } else if (c === "-" || /[0-9]/.test(c)) {
      const m = source.slice(i).match(/^-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/);
      if (!m) throw new Error("Invalid JSON number");
      const n = m[0];
      out +=
        /^-?\d+$/.test(n) && !Number.isSafeInteger(Number(n))
          ? JSON.stringify(n)
          : n;
      i += n.length;
    } else {
      out += c;
      i++;
    }
  }
  return JSON.parse(out);
}
function codexExecutionPolicy(provider,approval,sandbox){
  if(provider!=="codex") return undefined;
  if(!["untrusted","on-request","never"].includes(approval)||!["read-only","workspace-write","danger-full-access"].includes(sandbox)) throw new Error("Invalid Codex execution policy");
  return approval==="untrusted"&&sandbox==="read-only"?undefined:{approval_policy:approval,sandbox};
}
function mergeToolCall(call, patch) {
  for (const [key, value] of Object.entries(patch))
    if (value !== null && key !== "output_delta" && key !== "activity") call[key] = value;
  if (patch.activity?.length) call.activity = (call.activity || []).concat(patch.activity);
  if (patch.output_delta) call.output = (typeof call.output === "string" ? call.output : "") + patch.output_delta;
  return call;
}
function isWorkEvent(event) {
  return event.type === "verification" || event.type === "tool" || event.type === "input_delivery" ||
    (event.type === "runtime_status" && event.status === "retrying");
}
function verificationPassed(report) {
  return !!report.finished_revision && report.revision.head === report.finished_revision.head &&
    report.revision.fingerprint === report.finished_revision.fingerprint && report.checks.length > 0 &&
    report.checks.every((check) => check.status === "passed" && check.exit_code === 0);
}
function finishToolCall(call) {
  if (!call.status || ["pending", "in_progress"].includes(call.status)) call.status = "interrupted";
  return call;
}
function followsTranscriptTail(viewport) {
  return viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop <= 2;
}
function restoredTranscriptTop(viewport, previous) {
  return previous && !followsTranscriptTail(previous) ? previous.scrollTop : viewport.scrollHeight;
}
function questionAnswer(multiple, selected, freeText) {
  const text = freeText.trim();
  const values = text ? (multiple ? selected.concat(text) : [text]) : selected;
  const answer = [...new Set(values)];
  if (!answer.length || (!multiple && answer.length !== 1)) throw new Error("질문에 답변을 선택하거나 직접 입력하세요.");
  return answer;
}
class GuidanceSelection {
  constructor() { this.scope = ""; this.snapshot = null; this.selected = []; }
  begin(scope) {
    if (this.scope !== scope) { this.scope = scope; this.snapshot = null; this.selected = []; }
  }
  receive(scope, snapshot) {
    if (scope !== this.scope) return false;
    this.snapshot = snapshot;
    return true;
  }
  groups() {
    const groups=new Map();
    for (const skill of this.snapshot?.skills||[]) {
      const plugin=skill.id.startsWith("plugin:")?skill.id.slice(7).split("/")[0]:"";
      if (!groups.has(plugin)) groups.set(plugin,[]);
      groups.get(plugin).push(skill);
    }
    return [...groups].sort(([a],[b])=>a<b?-1:a>b?1:0);
  }
  toggle(id) {
    if (this.selected.some(skill=>skill.id===id)) { this.selected=this.selected.filter(skill=>skill.id!==id); return; }
    if (!this.snapshot?.skills.some(skill=>skill.id===id&&skill.enabled)) throw new Error("스킬 목록을 새로 고침하세요.");
    if (this.selected.length>=8) throw new Error("스킬은 한 턴에 최대 8개까지 선택할 수 있습니다.");
    this.selected=this.selected.concat({id});
  }
}
function skillInScope(id,scope) { return id.replace(/^plugin:/,"").startsWith(`${scope}:`); }
function skillScopeIds(skills,files,scope) {
  return [...new Set(skills.map(skill=>skill.id).concat(files))].filter(id=>id.endsWith("/SKILL.md")&&skillInScope(id,scope));
}
class GuidanceEditor {
  constructor() { this.scope=""; this.file=null; this.text=""; this.destination="repo"; }
  selectScope(destination) {
    if(this.dirty) return false;
    if(this.destination!==destination) { this.destination=destination; this.file=null; this.text=""; }
    return true;
  }
  get dirty() { return !!this.file && this.text!==this.file.text; }
  begin(scope) {
    if (this.scope!==scope) {
      if (this.dirty) return false;
      this.scope=scope; this.file=null; this.text="";
    }
    return true;
  }
  load(file) { this.file=file; this.text=file.text; }
  request() { return this.file ? {id:this.file.id,text:this.text,expected_revision:this.file.revision} : null; }
}
class PluginCatalogState {
  constructor() { this.scope=""; this.catalog=null; this.selected=null; this.query=""; this.removing=null; }
  begin(scope) {
    if (this.scope!==scope) { this.scope=scope; this.catalog=null; this.selected=null; this.removing=null; }
  }
  receive(scope,catalog) {
    if (scope!==this.scope) return false;
    this.catalog=catalog;
    if (!catalog.plugins.some(plugin=>plugin.id===this.selected)) this.selected=null;
    this.removing=null;
    return true;
  }
  visible(scope) {
    const query=this.query.trim().toLocaleLowerCase();
    return (this.catalog?.plugins||[]).filter(plugin=>(scope==="all"||plugin.id.startsWith(`${scope}:`))&&
      `${plugin.manifest.display_name||""} ${plugin.manifest.name} ${plugin.manifest.description}`.toLocaleLowerCase().includes(query));
  }
}
async function collectPluginFiles(picked,scope) {
  if (!['user','repo'].includes(scope)||!picked.length||picked.length>128) throw new Error("플러그인 폴더 하나를 선택하세요. 파일은 최대 128개입니다.");
  const root=picked[0].webkitRelativePath.split('/')[0], files=Object.create(null), names=new Set();
  let total=0;
  for (const file of picked) {
    const path=file.webkitRelativePath;
    if (!path.startsWith(`${root}/`)) throw new Error("플러그인 폴더 하나만 선택하세요.");
    const name=path.slice(root.length+1), parts=name.split('/');
    if (parts.length>6||parts.some(part=>!part||part==='.'||part==='..'||/[\\\x00-\x1f:*?"<>|]/.test(part)||/[. ]$/.test(part))) throw new Error("가져올 수 없는 플러그인 경로입니다.");
    if (names.has(name.toLowerCase())) throw new Error("중복된 플러그인 파일 이름입니다.");
    names.add(name.toLowerCase());
    if (file.size>65536||(total+=file.size)>1048576) throw new Error("플러그인은 파일당 64 KiB, 전체 1 MiB까지 가져올 수 있습니다.");
    const bytes=new Uint8Array(await file.arrayBuffer());
    if (bytes.length!==file.size) throw new Error("가져오는 중 파일 크기가 변경되었습니다.");
    files[name]=Array.from(bytes);
  }
  if (!Object.hasOwn(files,'plugin.json')) throw new Error("선택한 폴더에 plugin.json이 없습니다.");
  return {scope,files};
}
class GitReview {
  constructor() { this.begin(null); }
  begin(workspace) {
    this.workspace=workspace;this.overview=null;this.message="";this.paths=[];this.expected=null;
    this.clearDiff();
  }
  clearDiff() {
    this.diffRequest=(this.diffRequest || 0)+1;
    this.fileDiff=null;this.diffPath="";this.diffLoading=false;this.diffError="";
  }
  receive(workspace,overview) {
    if(workspace!==this.workspace || String(overview.workspace_id)!==String(workspace)) return false;
    this.overview=overview;
    this.message=overview.proposal?.message || "";
    this.paths=(overview.proposal?.paths || []).filter(path=>overview.files.some(file=>file.path===path));
    this.expected=overview.proposal?.expected || overview.version;
    this.clearDiff();
    return true;
  }
  beginDiff(path) {
    if(!this.overview?.files.some(file=>file.path===path)) throw new Error("현재 변경 파일만 검토하세요.");
    this.clearDiff();this.diffPath=path;this.diffLoading=true;
    return {workspace:this.workspace,path,overview:this.overview,request:this.diffRequest};
  }
  currentDiff(snapshot) {
    return snapshot.workspace===this.workspace && snapshot.overview===this.overview && snapshot.request===this.diffRequest;
  }
  receiveDiff(snapshot,diff) {
    if(!this.currentDiff(snapshot)) return false;
    if(String(diff.workspace_id)!==String(this.workspace) || diff.path!==snapshot.path) throw new Error("작업공간 또는 파일이 다른 diff 응답을 거절했습니다.");
    this.fileDiff=diff;this.diffLoading=false;return true;
  }
  get stale() {
    const version=this.overview?.version, expected=this.expected;
    return !version || !expected || ["head","index","worktree","branch"].some(key=>version[key]!==expected[key]);
  }
  reviewCurrent() { this.expected=this.overview?.version; }
  toggle(path,selected) {
    if(!this.overview?.files.some(file=>file.path===path)) throw new Error("현재 변경 파일만 선택하세요.");
    this.paths=this.paths.filter(value=>value!==path);
    if(selected) this.paths.push(path);
  }
  approve() {
    if(this.stale) throw new Error("변경 내용이 달라졌습니다. 현재 diff를 다시 검토하세요.");
    if(!this.paths.length) throw new Error("커밋할 파일을 선택하세요.");
    if(!this.message.trim()) throw new Error("커밋 메시지를 입력하세요.");
    return {type:"commit_selected",message:this.message,paths:this.paths.slice(),expected:this.expected};
  }
}
async function loadGitDiff(review,path,read,onChange) {
  const snapshot=review.beginDiff(path);onChange();
  try {review.receiveDiff(snapshot,await read(`/v1/workspaces/${snapshot.workspace}/diff?path=${encodeURIComponent(path)}`));}
  catch(e) {if(review.currentDiff(snapshot)){review.diffLoading=false;review.diffError=String(e.message || e);}}
  finally {if(review.currentDiff(snapshot)) onChange();}
}
function renderGitReview(container,review,context,busy,onCommand,onChange,onDiff=()=>{}) {
  container.replaceChildren();
  const doc=container.ownerDocument;
  const add=(parent,tag,text)=>{const node=doc.createElement(tag);node.textContent=text;parent.append(node);return node;};
  const action=(parent,text,enabled,callback,mutation=true)=>{
    const node=add(parent,"button",text);node.disabled=(mutation && busy) || !enabled;
    node.onclick=()=>{if(!node.disabled) callback();};return node;
  };
  const overview=review.overview;
  const branch=overview?.version.branch===undefined ? context.branch || "브랜치 없음" : overview.version.branch || "분리된 HEAD";
  add(container,"p",`${context.name || "프로젝트"} · ${branch} · ${context.path || ""}`);
  if(!overview) {add(container,"p","Git 상태를 새로고침하세요.");return;}
  if(overview.commit) add(container,"p",`커밋 완료: ${overview.commit}`);
  if(overview.backup) add(container,"p",`복구용 복원 지점: ${overview.backup}`);
  add(container,"h3",overview.proposal ? "에이전트 커밋 제안 검토" : "변경 파일 · 커밋");
  add(container,"p","선택한 파일의 현재 변경만 커밋합니다. diff와 메시지를 검토한 뒤 승인하세요.");
  if(review.stale) {
    add(container,"p","제안 이후 변경 내용이 달라졌습니다. 현재 diff를 다시 검토하세요.");
    action(container,"현재 변경으로 다시 검토",true,()=>{review.reviewCurrent();onChange();});
  }
  const files=add(container,"div","");files.className="git-files";
  for(const file of overview.files) {
    const row=add(files,"div","");row.className="tools";
    const label=add(row,"label",`${file.index_status}${file.worktree_status} ${file.path}`);label.className="git-file";
    const check=add(label,"input","");check.type="checkbox";check.value=file.path;
    check.checked=review.paths.includes(file.path);check.disabled=busy;
    check.onchange=()=>{if(!busy){review.toggle(file.path,check.checked);approve.disabled=review.stale || !review.paths.length || !review.message.trim();}};
    action(row,"변경 검토",true,()=>onDiff(file.path),false);
    action(row,"스테이징",!review.stale,()=>onCommand({type:"stage",path:file.path,staged:true,expected:overview.version}));
    action(row,"스테이징 취소",!review.stale && ![" ","?"].includes(file.index_status),()=>onCommand({type:"stage",path:file.path,staged:false,expected:overview.version}));
  }
  if(!overview.files.length) add(container,"p","변경 파일이 없습니다.");
  if(review.diffPath) {
    add(container,"h3",`파일 변경 검토 · ${review.diffPath}`);
    if(review.diffLoading) add(container,"p","파일 diff 확인 중…");
    if(review.diffError) add(container,"p",`diff 조회 실패: ${review.diffError}`);
    if(review.fileDiff) {
      if(review.fileDiff.untracked) add(container,"p","미추적 파일");
      for(const [title,text] of [["스테이징된 변경",review.fileDiff.staged],["스테이징되지 않은 변경",review.fileDiff.unstaged]]) {
        add(container,"h4",title);add(container,"pre",text || "변경 없음").className="git-diff";
      }
      if(review.fileDiff.truncated) add(container,"p","파일 diff가 길어 일부만 표시됩니다.");
    }
  }
  add(container,"h4","스테이징된 전체 변경");
  const diff=add(container,"pre",overview.diff || "표시할 diff가 없습니다.");diff.className="git-diff";
  if(overview.truncated) add(container,"p","diff가 길어 일부만 표시됩니다.");
  const label=add(container,"label","커밋 메시지");
  const message=add(label,"textarea","");message.value=review.message;message.rows=3;message.maxLength=8192;message.disabled=busy;
  const approve=action(container,"선택한 파일 커밋 승인",!review.stale && review.paths.length>0 && !!review.message.trim(),()=>onCommand(review.approve()));
  message.oninput=()=>{review.message=message.value;approve.disabled=busy || review.stale || !review.paths.length || !review.message.trim();};
  const checkpoints=add(container,"details","");add(checkpoints,"summary","작업 상태 저장 · 복원");
  add(checkpoints,"p","현재 변경 파일과 스테이징 상태를 저장합니다. 복원 전에는 되돌릴 내용을 확인합니다.");
  const checkpointLabel=add(checkpoints,"label","복원 지점 이름");
  const name=add(checkpointLabel,"input","");name.maxLength=256;name.disabled=busy;
  const save=action(checkpoints,"복원 지점 저장",false,()=>onCommand({type:"checkpoint",label:name.value}));
  name.oninput=()=>{save.disabled=busy || !name.value.trim();};
  for(const checkpoint of overview.checkpoints) {
    const row=add(checkpoints,"div","");add(row,"p",`${checkpoint.label} · ${checkpoint.id}`);
    action(row,"되돌리기 미리보기",checkpoint.head===overview.version.head,()=>onCommand({type:"preview_restore",checkpoint:checkpoint.id}));
  }
}
// Every file the operator would be installing, in review order: manifest first.
function skillProposalFiles(proposal) {
  const root = proposal.skill_id.replace(/^repo:/, "").replace(/\/SKILL\.md$/, "");
  return [
    { path: `${root}/SKILL.md`, text: proposal.content },
    ...Object.keys(proposal.resources).sort().map((name) => ({ path: `${root}/${name}`, text: proposal.resources[name] })),
  ];
}
// What an MCP proposal would run or connect to, one fact per line.
function mcpProposalLines(proposal) {
  const server = proposal.server;
  const lines = [`이름: ${server.name}`];
  if (server.command !== undefined) {
    lines.push(`실행 파일: ${server.command}`);
    if (server.args?.length) lines.push(`인자: ${server.args.join(" ")}`);
  } else {
    lines.push(`주소: ${server.url}`);
    if (server.bearer_token_env_var) lines.push(`Bearer 환경 변수: ${server.bearer_token_env_var}`);
    for (const [header, variable] of Object.entries(server.env_http_headers || {})) lines.push(`헤더 환경 변수: ${header}=${variable}`);
  }
  for (const check of proposal.environment || []) lines.push(`${check.ready ? "환경 변수 준비됨" : "환경 변수 미설정"}: ${check.variable}`);
  lines.push(`설치 위치: ${proposal.plugin_id}`, `이유: ${proposal.rationale}`);
  return lines;
}
// What the user last asked in the proposing conversation, as recorded by the host.
function proposalRequestLine(proposal) {
  return `대화의 마지막 요청: ${proposal.requested_during ?? "(확인할 수 없음 — 에이전트가 스스로 제안했을 수 있습니다)"}`;
}
if (typeof module !== "undefined") module.exports = { proposalRequestLine, skillProposalFiles, mcpProposalLines,GitReview,renderGitReview,loadGitDiff,PluginCatalogState,collectPluginFiles,GuidanceEditor,GuidanceSelection,skillInScope,skillScopeIds,parseJson,codexExecutionPolicy,mergeToolCall,isWorkEvent,verificationPassed,finishToolCall,followsTranscriptTail,restoredTranscriptTop,questionAnswer };
if (typeof document !== "undefined") start();
function start() {
  const $ = (id) => document.getElementById(id);
  const transcript = $("messages"), jumpLatest = $("jumpLatest");
  let workCollapsed = false;
  function renderWorkPanel() {
    const active = $("timeline").childElementCount > 0;
    $("workToggle").hidden = !active;
    $("workToggle").textContent = busy ? "작업 중 · 실행 내역" : "작업 내역";
    $("workToggle").setAttribute("aria-expanded", String(active && !workCollapsed));
    $("workPanel").hidden = !active || workCollapsed;
    $("columns").classList.toggle("work-open", active && !workCollapsed);
  }
  bind("workToggle", () => { workCollapsed = !workCollapsed; renderWorkPanel(); });
  bind("workClose", () => { workCollapsed = true; renderWorkPanel(); $("workToggle").focus(); });
  function updateJump() { jumpLatest.hidden = followsTranscriptTail(transcript); }
  function updateReply(reply, text) {
    const follow = followsTranscriptTail(transcript);
    reply.textContent = text;
    if (follow) transcript.scrollTop = transcript.scrollHeight;
    updateJump();
  }
  transcript.addEventListener("scroll", updateJump);
  bind("jumpLatest", () => {
    transcript.scrollTop = transcript.scrollHeight;
    updateJump();
  });
  let token = "",
    infos = [],
    conversation = null,
    attachments = [],
    busy = false,
    operation = null,
    job = null,
    frame = null,
    controller = null;
  let projectCatalog = {projects:[]};
  const gitReview=new GitReview();
  let gitLoading=false,gitRequest=0,gitRestore=null,gitFailure=false;
  const gitWorkspace=()=>String($("workspace").value || projectCatalog.default_workspace_id || "");
  function gitContext() {
    for(const project of projectCatalog.projects) {
      const workspace=project.workspaces.find(workspace=>String(workspace.id)===gitWorkspace());
      if(workspace) return {...workspace,name:project.name,git:project.git};
    }
    return null;
  }
  function renderGit() {
    const context=gitContext();
    $("projectGit").hidden=!context?.git;
    $("gitRefresh").disabled=gitLoading;
    $("commitProposal").hidden=!gitReview.overview?.proposal || gitReview.workspace!==gitWorkspace();
    $("gitReviewOpen").disabled=gitLoading;
    if(gitRestore) {
      const container=$("gitReview");container.replaceChildren();
      const description=document.createElement("p");description.textContent="복원하면 현재 변경이 대체됩니다. 아래 diff를 검토한 뒤 승인하세요.";
      const diff=document.createElement("pre");diff.className="git-diff";diff.textContent=gitRestore.overview.diff;
      container.append(description,diff);
      const preview=gitRestore;
      button(container,"복원 승인",()=>gitCommand({type:"restore",checkpoint:preview.checkpoint,expected:preview.overview.version})).disabled=busy || gitLoading;
      button(container,"복원 취소",()=>{gitRestore=null;renderGit();}).disabled=busy || gitLoading;
    } else renderGitReview($("gitReview"),gitReview,context || {},busy || gitLoading || gitFailure,gitCommand,renderGit,inspectGitFile);
  }
  async function inspectGitFile(path) {
    if(!token || gitReview.workspace!==gitWorkspace()) return;
    try {await loadGitDiff(gitReview,path,api,renderGit);} catch(e) {error(e);}
  }
  async function refreshGit() {
    const workspace=gitWorkspace(),requestId=++gitRequest;
    if(gitReview.workspace!==workspace) gitReview.begin(workspace);
    gitRestore=null;gitFailure=false;
    if(!token || !gitContext()?.git) {gitReview.begin(workspace);gitLoading=false;renderGit();return;}
    gitLoading=true;$("gitStatus").textContent="Git 상태 확인 중…";renderGit();
    try {
      const overview=await api(`/v1/workspaces/${workspace}/git`);
      if(requestId!==gitRequest || workspace!==gitWorkspace() || !token) return;
      if(!gitReview.receive(workspace,overview)) throw new Error("작업공간이 다른 Git 응답을 거절했습니다.");
      $("gitStatus").textContent="";
    } catch(e) {
      if(requestId===gitRequest) {gitFailure=true;$("gitStatus").textContent=String(e.message || e);}
    } finally {
      if(requestId===gitRequest) {gitLoading=false;renderGit();}
    }
  }
  async function gitCommand(command) {
    if(busy || gitLoading || gitFailure || !token || gitReview.workspace!==gitWorkspace()) return;
    const workspace=gitWorkspace(),requestId=++gitRequest;
    gitLoading=true;setBusy(true);$("gitStatus").textContent="Git 작업 중…";
    try {
      const overview=await api(`/v1/workspaces/${workspace}/git`,"POST",command);
      if(requestId!==gitRequest || workspace!==gitWorkspace() || !token) return;
      if(String(overview.workspace_id)!==workspace) throw new Error("작업공간이 다른 Git 응답을 거절했습니다.");
      if(command.type==="preview_restore") gitRestore={checkpoint:command.checkpoint,overview};
      else {gitRestore=null;gitReview.receive(workspace,overview);}
      $("gitStatus").textContent=overview.commit ? `커밋 완료: ${overview.commit}` : "Git 작업이 완료되었습니다.";
    } catch(e) {
      if(requestId===gitRequest) {gitFailure=true;gitRestore=null;$("gitStatus").textContent=`${e.message || e} · Git 상태를 새로고침하세요.`;}
    } finally {
      if(requestId===gitRequest) {gitLoading=false;setBusy(false);renderGit();}
    }
  }
  bind("gitRefresh",refreshGit);
  bind("gitReviewOpen",()=>{$("projectGit").open=true;$("projectGit").scrollIntoView({block:"start"});$("gitRefresh").focus();});
  function verificationProject() {
    const selected = Number($("workspace").value || projectCatalog.default_workspace_id);
    return projectCatalog.projects.find((project) => project.workspaces.some((workspace) => workspace.id === selected));
  }
  function renderVerificationSettings() {
    const project = verificationProject();
    $("projectVerification").hidden = !project;
    if (!project) return;
    const plan = project.verification || {commands:[],max_attempts:3,timeout_seconds:300};
    $("verificationProject").textContent = `${project.name} · ${plan.commands.length ? `필수 명령 ${plan.commands.length}개 · 최대 ${plan.max_attempts}회 · 명령당 ${plan.timeout_seconds}초` : "필수 검증 미설정"}`;
    $("verificationCommands").value = JSON.stringify(plan.commands,null,2);
  }
  bind("verificationSave", async () => {
    const project = verificationProject();
    if (!project || busy) return;
    const commands = parseJson($("verificationCommands").value);
    setBusy(true);
    try {
      projectCatalog = await api(`/v1/projects/${project.id}/verification`,"PUT",{...(project.verification || {}),commands});
      renderVerificationSettings();
    } finally { setBusy(false); }
  });
  const scope = () => ({
    provider: $("provider").value,
    account: $("account").value || "default",
    workspace: $("workspace").value,
  });
  const endpoint = () => `/v1/providers/${scope().provider}`;
  const guidanceSelection = new GuidanceSelection();
  async function refreshGuidance() {
    const key = JSON.stringify(scope());
    guidanceSelection.begin(key);
    renderGuidance();
    try {
      const snapshot = await api("/v1/guidance");
      if (guidanceSelection.receive(key,snapshot)) {
        renderGuidance();
        renderGuidanceEditor();
        await refreshPlugins();
      }
    } catch (e) {
      if (key===guidanceSelection.scope) $("guidanceStatus").textContent=`조회 실패 · ${e.message} · 이전 목록은 오래되었을 수 있습니다.`;
    }
  }
  function renderGuidance() {
    const snapshot=guidanceSelection.snapshot;
    $("skills").replaceChildren();
    $("selectedSkills").replaceChildren();
    for(const reference of guidanceSelection.selected) button($("selectedSkills"),`${reference.id} · 제거`,()=>{
      if (!busy) { guidanceSelection.toggle(reference.id); renderGuidance(); }
    }).disabled=busy;
    $("skillsSummary").textContent="플러그인";
    $("guidanceStatus").textContent="지침 확인 필요";
    $("skillsCatalog").replaceChildren();
    if (!snapshot) { $("skillsCatalog").textContent="스킬 목록을 확인 중입니다."; return; }
    const app=snapshot.instructions.some(doc=>doc.source.startsWith("user:"));
    const project=snapshot.instructions.some(doc=>doc.source.startsWith("repo:"));
    $("guidanceStatus").textContent=snapshot.diagnostics.length?"지침 확인 필요 · 진단 있음":app&&project?"앱 공통·프로젝트 지침 적용 중":app?"앱 공통 지침 적용 중":project?"프로젝트 지침 적용 중":"적용할 지침 없음";
    $("guidanceDiagnostics").textContent=((editingSkills?[]:snapshot.instructions.map(doc=>`AGENTS · ${doc.source}${doc.truncated?" [truncated]":""}`)).concat(snapshot.diagnostics.map(d=>`${d.source} · ${d.message}`))).join("\n");
    const suffix=$("prompt").value.match(/(?:^|\s)(\$[^\s]*)$/)?.[1];
    const info=infos.find(info=>info.provider===scope().provider);
    const support=document.createElement("small");
    support.textContent=!info?.available?"제공자 런타임 확인 필요":info.capabilities?.mcp_capability_check?"직접 선택 가능 · 자동 사용은 런타임 확인 필요":(info.capabilities?.mcp_stdio||info.capabilities?.mcp_http)?"직접 선택 가능 · 호스트 연결 시 자동 사용 가능":"직접 선택 가능 · 자동 사용 지원 미확인";
    $("skills").append(support);
    const groups=new Map();
    for (const [id,skills] of guidanceSelection.groups()) {
      const plugin=pluginState.scope===pluginKey()?pluginState.catalog?.plugins.find(plugin=>plugin.id===id):null;
      const title=id?(plugin?`${plugin.manifest.display_name||plugin.manifest.name} · ${id}`:id):"개별 스킬";
      const group=document.createElement("fieldset"), legend=document.createElement("legend");
      group.setAttribute("aria-label",title); legend.textContent=title; group.append(legend);
      for (const skill of skills) groups.set(skill.id,group);
    }
    for(const skill of snapshot.skills) {
      const row=document.createElement("div"), label=document.createElement("label"), choice=document.createElement("input"), description=document.createElement("small");
      choice.type="checkbox";
      choice.checked=guidanceSelection.selected.some(reference=>reference.id===skill.id);
      choice.disabled=busy||(!skill.enabled&&!choice.checked);
      choice.addEventListener("change",()=>{ try { if(!suffix || !guidanceSelection.selected.some(reference=>reference.id===skill.id)) guidanceSelection.toggle(skill.id); if(suffix) $("prompt").value=$("prompt").value.slice(0,-suffix.length); renderGuidance(); } catch(e) { error(e); choice.checked=!choice.checked; } });
      label.append(choice,document.createTextNode(`${skill.display_name||skill.name} · ${skill.id}`));
      description.textContent=skill.description;
      row.append(label,description);
      if(!suffix || (skill.enabled && `${skill.name} ${skill.description}`.toLowerCase().includes(suffix.slice(1).toLowerCase()))) groups.get(skill.id).append(row);
      if(!skillInScope(skill.id,guidanceEditor.destination)) continue;
      const packaged=skill.id.startsWith('plugin:');
      const setting=document.createElement("div");
      const name=document.createElement("span");
      name.textContent=`${skill.display_name||skill.name} · ${skill.id} `;
      setting.append(name);
      button(setting,skill.enabled?"공통 사용 끄기":"공통 사용 켜기",async()=>{
        if(busy) return;
        setBusy(true);
        try { await api("/v1/skills/enabled","PUT",{id:skill.id,enabled:!skill.enabled}); await refreshGuidance(); }
        finally { setBusy(false); }
      }).disabled=busy;
      const summary=document.createElement("p");
      summary.textContent=`${packaged?"플러그인 · ":""}${skill.description} · ${skill.allow_implicit_invocation?"자동 사용 허용":"직접 선택으로 사용"}`;
      setting.append(summary);
      button(setting,"자세히 · 편집",()=>openGuidanceFile(skill.id)).disabled=busy||guidanceEditor.dirty||packaged;
      if (`${skill.name} ${skill.description} ${skill.id}`.toLowerCase().includes($("skillSearch").value.toLowerCase())) $("skillsCatalog").append(setting);
    }
    for (const group of new Set(groups.values())) if(group.children.length>1) $("skills").append(group);
    if(!$("skillsCatalog").childElementCount) $("skillsCatalog").textContent=$("skillSearch").value?"검색 결과가 없습니다.":skillScopeIds(snapshot.skills,guidanceFiles.files,guidanceEditor.destination).length?"아래 파일을 열어 스킬 내용을 확인하세요.":guidanceEditor.destination==="user"?"앱 공통에 등록된 스킬이 없습니다.":"이 프로젝트에 등록된 스킬이 없습니다.";
  }
  $("prompt").addEventListener("input",()=>{
    if(/(?:^|\s)\$[^\s]*$/.test($("prompt").value)) { $("guidance").open=true; }
    renderGuidance();
  });
  $("guidance").addEventListener("toggle",()=>{ if($("guidance").open && token) refreshGuidance().catch(error); });
  const guidanceEditor = new GuidanceEditor();
  let guidanceFiles = {files:[],project_prefix:""};
  let guidanceFilesScope = null;
  let editingSkills = false;
  let creatingSkill = false;
  const editorScope = () => String(scope().workspace || projectCatalog.default_workspace_id || "");
  function renderGuidanceEditor() {
    const user=guidanceEditor.destination==="user";
    const project=verificationProject();
    $("skillScopeTabs").hidden=!editingSkills;
    $("skillScopeContext").hidden=!editingSkills;
    $("skillsCatalog").hidden=!editingSkills||!!guidanceEditor.file;
    $("guidanceScopeField").hidden=editingSkills;
    $("guidanceScope").value=guidanceEditor.destination;
    for(const [id,destination,label] of [["skillScopeUser","user","앱 공통"],["skillScopeProject","repo","현재 프로젝트"]]) {
      $(id).textContent=`${label} · ${skillScopeIds(guidanceSelection.snapshot?.skills||[],guidanceFiles.files,destination).length}`;
      $(id).setAttribute("aria-pressed",String(destination===guidanceEditor.destination));
    }
    $("skillScopeTitle").textContent=user?"앱 공통":`현재 프로젝트 · ${project?.name||"프로젝트 없음"}`;
    $("skillScopeDescription").textContent=user?"모든 프로젝트에서 사용할 수 있습니다.":project?"이 프로젝트에서만 사용할 수 있습니다.":"프로젝트를 열면 프로젝트 스킬을 관리할 수 있습니다.";
    $("skillNew").textContent=user?"앱 공통 스킬 추가":"이 프로젝트에 스킬 추가";
    $("skillFolderLabel").textContent=user?"앱 공통에 폴더 가져오기":"이 프로젝트에 폴더 가져오기";
    $("skillSearchField").hidden=!editingSkills;
    $("skillFolderField").hidden=!editingSkills||!!guidanceEditor.file;
    $("skillNew").hidden=!editingSkills || !!guidanceEditor.file;
    $("skillDescriptionField").hidden=!editingSkills||!creatingSkill||!!guidanceEditor.file;
    $("guidanceNameField").hidden=!editingSkills||!creatingSkill||!!guidanceEditor.file;
    $("guidanceNewSkill").hidden=!editingSkills||!creatingSkill||!!guidanceEditor.file;
    $("guidanceNewAgents").hidden=editingSkills;
    $("guidanceDescription").textContent=editingSkills?"스킬(SKILL.md)을 생성·편집하고 공통 활성 상태를 관리합니다. 대화에서 사용할 스킬을 선택하세요.":"앱 공통·프로젝트 지침(AGENTS.md)을 관리합니다. 다음 턴에 자동 적용됩니다.";
    $("guidanceLimit").textContent=`${editingSkills?"SKILL.md 64":"AGENTS.md 32"} KiB · UTF-8 Markdown. 업로드 후 저장을 눌러 적용하세요.`;
    options("guidanceFile",[["","파일 선택"],...guidanceFiles.files.filter(id=>id.endsWith("/SKILL.md")===editingSkills&&(!editingSkills||skillInScope(id,guidanceEditor.destination))).map(id=>[id,id])],guidanceEditor.file?.id || "");
    const same = guidanceEditor.scope===editorScope();
    const locked = busy || guidanceEditor.dirty || !same || guidanceFilesScope!==editorScope();
    $("skillScopeUser").disabled=locked;
    $("skillScopeProject").disabled=locked;
    $("skillNew").disabled=locked||(!user&&!project);
    $("skillFolder").disabled=locked||(!user&&!project);
    for (const id of ["guidanceFile","guidanceScope","guidanceName","guidanceNewAgents","guidanceNewSkill"])
      $(id).disabled=locked;
    $("guidanceFilesRefresh").disabled=busy || guidanceEditor.dirty;
    $("guidanceNewSkill").disabled=locked || (!user&&!project) || !$("guidanceName").value.trim() || !$("skillDescription").value.trim();
    $("guidanceEdit").hidden=!guidanceEditor.file;
    if ($("guidanceText").value!==guidanceEditor.text) $("guidanceText").value=guidanceEditor.text;
    $("guidanceText").disabled=busy;
    $("guidanceSave").disabled=busy || !same || !guidanceEditor.file || (!guidanceEditor.dirty && guidanceEditor.file.revision!==null);
    $("guidanceUpload").disabled=busy || !same || !guidanceEditor.file;
    $("guidanceUploadLabel").textContent=guidanceEditor.dirty?"초안을 교체할 Markdown 파일 선택":"Markdown 파일 업로드";
    $("guidanceDiscard").disabled=busy;
    $("guidanceTarget").textContent=guidanceEditor.file?.id || "";
    $("guidanceDraftStatus").textContent=!same?"작업공간이 변경되었습니다. 초안을 복사하거나 편집 취소 후 다시 불러오세요.":guidanceEditor.dirty?"저장하지 않은 변경이 있습니다.":"";
  }
  async function refreshGuidanceFiles() {
    const key=editorScope();
    if (!guidanceEditor.begin(key)) { renderGuidanceEditor(); return; }
    guidanceFilesScope=null;
    renderGuidanceEditor();
    const catalog=await api("/v1/guidance/files");
    if (key!==editorScope()) return;
    guidanceFiles=catalog;
    guidanceFilesScope=key;

    renderGuidanceEditor();
  }
  async function openGuidanceFile(id) {
    if (busy || guidanceEditor.dirty || !guidanceEditor.begin(editorScope())) return;
    const key=editorScope();
    setBusy(true);
    try {
      const file=await api("/v1/guidance/files/read","POST",{id});
      if (key!==editorScope()) return;
      guidanceEditor.load(file);
      if (file.revision===null && id.endsWith("/SKILL.md")) guidanceEditor.text=`---\nname: ${JSON.stringify($("guidanceName").value.trim())}\ndescription: ${JSON.stringify($("skillDescription").value.trim())}\n---\n`;
      creatingSkill=false;
      $("guidanceEditorStatus").textContent="";
    } finally { setBusy(false); renderGuidanceEditor(); }
  }
  function newGuidanceId(skill) {
    const user=guidanceEditor.destination==="user";
    return `${user?"user:":"repo:"}${user?"":guidanceFiles.project_prefix}${skill?`${user?"":".agents/"}skills/${$("guidanceName").value.trim()}/SKILL.md`:"AGENTS.md"}`;
  }
  for (const [id, skills] of [["guidanceSettings",false],["skillsSettings",true]]) {
    $(id).addEventListener("toggle",async()=>{
      if (!$(id).open || !token) return;
      const previous=editingSkills?"skillsSettings":"guidanceSettings";
      if (skills!==editingSkills && (guidanceEditor.dirty || busy)) {
        $(id).open=false;
        $(previous).open=true;
        $("guidanceEditorStatus").textContent=busy?"처리 중입니다. 완료 후 다시 선택하세요.":"저장하지 않은 변경이 있습니다. 저장하거나 편집을 취소하세요.";
        return;
      }
      if (skills!==editingSkills) { guidanceEditor.file=null; guidanceEditor.text=""; $("guidanceEditorStatus").textContent=""; }
      editingSkills=skills;
      if (previous!==id) $(previous).open=false;
      $(id).append($("guidanceEditor"));
      try { await refreshGuidanceFiles(); await refreshGuidance(); } catch(e) { error(e); }
    });
  }
  bind("guidanceFilesRefresh",async()=>{ await refreshGuidanceFiles(); await refreshGuidance(); });
  function selectSkillScope(destination) {
    if(busy || !guidanceEditor.selectScope(destination)) return;
    creatingSkill=false;
    for(const id of ["skillSearch","guidanceName","skillDescription","skillFolder"]) $(id).value="";
    $("guidanceEditorStatus").textContent="";
    renderGuidanceEditor(); renderGuidance();
  }
  bind("skillScopeUser",()=>selectSkillScope("user"));
  bind("skillScopeProject",()=>selectSkillScope("repo"));
  $("guidanceScope").addEventListener("change",()=>selectSkillScope($("guidanceScope").value));
  bind("guidanceNewAgents",()=>openGuidanceFile(newGuidanceId(false)));
  $("skillSearch").addEventListener("input",renderGuidance);
  $("skillFolder").addEventListener("change",async()=>{
    const key=editorScope();
    try {
      if(busy||guidanceEditor.dirty) return;
      const picked=Array.from($("skillFolder").files);
      if(!picked.length) return;
      const folder=picked[0].webkitRelativePath.split("/")[0];
      const files={}; let total=0;
      if(picked.length>128) throw new Error("스킬 파일은 최대 128개입니다.");
      for(const file of picked) {
        if(file.size>65536 || (total+=file.size)>1048576) throw new Error("스킬 폴더 크기 제한을 초과했습니다.");
        if(!file.webkitRelativePath.startsWith(folder+"/")) throw new Error("스킬 폴더 하나를 선택하세요.");
        files[file.webkitRelativePath.slice(folder.length+1)]=Array.from(new Uint8Array(await file.arrayBuffer()));
      }
      if(key!==editorScope()) throw new Error("작업공간이 변경되었습니다.");
      setBusy(true);
      const id=guidanceEditor.destination==="user"?`user:skills/${folder}/SKILL.md`:`repo:${guidanceFiles.project_prefix}.agents/skills/${folder}/SKILL.md`;
      await api("/v1/skills/import","POST",{id,files});
      await refreshGuidanceFiles(); await refreshGuidance();
    } catch(e) { error(e); } finally { $("skillFolder").value=""; setBusy(false); }
  });
  bind("skillNew",()=>{ creatingSkill=true; renderGuidanceEditor(); });
  $("skillDescription").addEventListener("input",renderGuidanceEditor);
  bind("guidanceNewSkill",()=>openGuidanceFile(newGuidanceId(true)));
  $("guidanceFile").addEventListener("change",()=>{ if($("guidanceFile").value) openGuidanceFile($("guidanceFile").value).catch(error); });
  $("guidanceName").addEventListener("input",renderGuidanceEditor);
  $("guidanceText").addEventListener("input",()=>{ guidanceEditor.text=$("guidanceText").value; $("guidanceEditorStatus").textContent=""; renderGuidanceEditor(); });
  $("guidanceUpload").addEventListener("change",async()=>{
    const upload=$("guidanceUpload").files[0], key=editorScope(), target=guidanceEditor.file?.id, draft=guidanceEditor.text;
    if (!upload) return;
    try {
      if(upload.size>65536) throw new Error("Markdown 파일은 최대 64 KiB입니다.");
      const text=new TextDecoder("utf-8",{fatal:true}).decode(await upload.arrayBuffer());
      if(key!==editorScope() || target!==guidanceEditor.file?.id || draft!==guidanceEditor.text) return;
      guidanceEditor.text=text;
      $("guidanceEditorStatus").textContent="파일을 불러왔습니다. 저장을 눌러 적용하세요.";
      renderGuidanceEditor();
    } catch(e) { error(e); } finally { $("guidanceUpload").value=""; }
  });
  bind("guidanceSave",async()=>{
    if(busy || guidanceEditor.scope!==editorScope()) return;
    const body=guidanceEditor.request();
    if(!body) return;
    setBusy(true);
    try {
      guidanceEditor.load(await api("/v1/guidance/files","PUT",body));
      $("guidanceEditorStatus").textContent="저장되었습니다. 다음 턴부터 반영됩니다.";
      await refreshGuidanceFiles(); await refreshGuidance();
    } finally { setBusy(false); renderGuidanceEditor(); }
  });
  bind("guidanceDiscard",async()=>{ guidanceEditor.file=null; guidanceEditor.text=""; $("guidanceEditorStatus").textContent=""; await refreshGuidanceFiles(); });
  const pluginState=new PluginCatalogState();
  let pluginTab="installed", pluginLoading=false, pluginMessage="", pluginFailure="";
  let mcpServers=null, mcpLoaded=false, mcpDirty=false;
  const pluginKey=()=>JSON.stringify(scope());
  function renderPlugins() {
    const same=pluginState.scope===pluginKey();
    const locked=busy||pluginLoading||!same;
    for (const id of ["pluginRefresh","pluginFolder","pluginScope"]) $(id).disabled=locked;
    $("pluginStatus").textContent=pluginLoading?"플러그인을 불러오는 중…":pluginMessage||
      (pluginState.catalog?`설치된 플러그인 ${pluginState.catalog.plugins.length}개 · 활성화하면 다음 턴부터 적용됩니다.`:"플러그인 목록을 불러오세요.");
    $("pluginError").textContent=pluginFailure;
    $("pluginDiagnostics").replaceChildren();
    for (const diagnostic of pluginState.catalog?.diagnostics||[]) {
      const item=document.createElement('p');item.textContent=`${diagnostic.source} · ${diagnostic.message} `;
      if (/^(user|repo):[a-z0-9][a-z0-9-]{0,63}$/.test(diagnostic.source)) button(item,'비활성화',()=>changePlugin('/v1/plugins/enabled','PUT',{id:diagnostic.source,enabled:false},'비활성화했습니다. 폴더 내용을 확인한 뒤 다시 가져오세요.')).disabled=locked;
      $("pluginDiagnostics").append(item);
    }
    const detail=pluginState.catalog?.plugins.find(p=>p.id===pluginState.selected);
    $("pluginCards").hidden=!!detail;
    $("pluginDetail").hidden=!detail;
    $("pluginCards").replaceChildren();
    $("pluginDetail").replaceChildren();
    if (!same||!pluginState.catalog) return;
    const name=p=>p.manifest.display_name||p.manifest.name;
    const scopeLabel=p=>p.id.startsWith('user:')?"앱 공통":"현재 프로젝트";
    const counts=p=>`스킬 ${p.skills.length}개 · MCP 연결 ${p.manifest.mcp_servers.length}개`;
    function text(parent,tag,value,className) {
      const node=document.createElement(tag); node.textContent=value;
      if (className) node.className=className;
      parent.append(node);
    }
    if (detail) {
      const panel=$("pluginDetail");
      button(panel,"← 설치된 플러그인",()=>{pluginState.selected=null;pluginState.removing=null;renderPlugins();}).disabled=locked;
      text(panel,'h2',name(detail));
      text(panel,'p',detail.manifest.description);
      text(panel,'small',`${scopeLabel(detail)}${detail.manifest.version?` · v${detail.manifest.version}`:''} · ${detail.enabled?'활성화됨':'비활성화됨'}`);
      text(panel,'p',counts(detail));
      button(panel,detail.enabled?"비활성화":"활성화",()=>changePlugin('/v1/plugins/enabled','PUT',{id:detail.id,enabled:!detail.enabled},detail.enabled?"비활성화되었습니다.":"활성화되었습니다. 다음 턴부터 적용됩니다.")).disabled=locked;
      text(panel,'h3','포함된 스킬');
      if (!detail.skills.length) text(panel,'p','포함된 스킬이 없습니다.');
      for (const skill of detail.skills) { text(panel,'h4',skill.display_name||skill.name); text(panel,'p',skill.description); }
      text(panel,'h3','도구 연결');
      if (!detail.manifest.mcp_servers.length) text(panel,'p','MCP 연결이 필요하지 않습니다.');
      for (const server of detail.manifest.mcp_servers) text(panel,'p',`${server.name} · ${detail.enabled?(server.oauth?'계정 연결 상태를 확인하세요':'활성화됨 · 연결은 실행 시 확인'):'플러그인 비활성화됨'}`);
      if (detail.manifest.mcp_servers.length) button(panel,"MCP 연결 · 인증 관리",()=>showPluginTab('mcp')).disabled=locked;
      const remove=document.createElement('div'); remove.className='plugin-remove'; panel.append(remove);
      if (pluginState.removing===detail.id) {
        text(remove,'p','설치된 복사본을 삭제할까요? 가져온 원본 폴더와 기존 Skill·MCP 설정은 유지됩니다.');
        button(remove,'삭제 확인',()=>changePlugin('/v1/plugins/remove','POST',{id:detail.id},'플러그인을 삭제했습니다.')).disabled=locked;
        button(remove,'취소',()=>{pluginState.removing=null;renderPlugins();}).disabled=locked;
      } else button(remove,'플러그인 삭제',()=>{pluginState.removing=detail.id;renderPlugins();}).disabled=locked;
    } else {
      for (const plugin of pluginState.visible($("pluginFilter").value)) {
        const card=document.createElement('article'); card.className='plugin-card';
        text(card,'span',`${scopeLabel(plugin)} · ${plugin.enabled?'활성화됨':'비활성화됨'}`,'plugin-badge');
        text(card,'h3',name(plugin)); text(card,'p',plugin.manifest.description);
        text(card,'small',counts(plugin));
        button(card,'자세히 보기',()=>{pluginState.selected=plugin.id;renderPlugins();$("pluginDetail").querySelector('button')?.focus();}).disabled=locked;
        $("pluginCards").append(card);
      }
      if (!$("pluginCards").childElementCount) {
        const empty=document.createElement('div'); empty.className='plugin-empty';
        text(empty,'h3',pluginState.catalog.plugins.length?'검색 결과가 없습니다':'플러그인으로 작업 도구를 모아보세요');
        text(empty,'p',pluginState.catalog.plugins.length?'검색어나 표시 범위를 바꿔보세요.':'plugin.json이 있는 로컬 폴더를 가져오세요. 기존 스킬과 MCP 설정도 위 탭에서 관리할 수 있습니다.');
        $("pluginCards").append(empty);
      }
    }
  }
  async function refreshPlugins() {
    const key=pluginKey(); pluginState.begin(key);
    pluginLoading=true; pluginFailure=""; renderPlugins();
    try {
      const catalog=await api('/v1/plugins');
      if (key===pluginKey()) pluginState.receive(key,catalog);
    } catch(e) { if(key===pluginKey()) pluginFailure=e.message; }
    finally { if(key===pluginKey()) {pluginLoading=false;renderPlugins();renderGuidance();} }
  }
  async function changePlugin(path,method,body,message) {
    if (busy||pluginLoading||pluginState.scope!==pluginKey()) return;
    const key=pluginKey(); setBusy(true); pluginFailure="";
    try {
      const catalog=await api(path,method,body);
      if (key!==pluginKey()) return;
      pluginState.receive(key,catalog); pluginMessage=message;
      await refreshGuidance();
      if (mcpLoaded) renderMcpConnections();
    } catch(e) { if(key===pluginKey()) pluginFailure=e.message; }
    finally { setBusy(false);renderPlugins(); }
  }
  async function showPluginTab(tab) {
    if (busy) return;
    if (guidanceEditor.dirty&&tab!==pluginTab) throw new Error('스킬 변경을 저장하거나 편집을 취소하세요.');
    pluginTab=tab;
    for (const [name,id] of [['installed','pluginInstalled'],['skills','pluginSkills'],['mcp','pluginMcp']]) {
      $(id).hidden=tab!==name;
      $(`plugin${name==='installed'?'Installed':name==='skills'?'Skills':'Mcp'}Tab`).setAttribute('aria-pressed',String(tab===name));
    }
    if (tab==='skills') { $("skillsSettings").open=true; }
    else if (tab==='installed') await refreshPlugins();
    else { await refreshPlugins(); if(!mcpDirty) await loadMcp(); else renderMcpConnections(); }
  }
  for (const tab of ['installed','skills','mcp']) bind(`plugin${tab==='installed'?'Installed':tab==='skills'?'Skills':'Mcp'}Tab`,()=>showPluginTab(tab));
  $("pluginSettings").addEventListener('toggle',()=>{if($("pluginSettings").open&&token) showPluginTab(pluginTab).catch(error);});
  bind('pluginRefresh',()=>{pluginMessage="";return refreshPlugins();});
  bind('pluginSyncClaude',async()=>{
    if(busy||pluginLoading) return;
    const key=pluginKey(); setBusy(true); pluginFailure="";
    try {
      const result=await api('/v1/plugins/claude','POST',{scope:$("pluginScope").value});
      if(key!==pluginKey()) return;
      pluginState.receive(key,result.catalog); pluginState.selected=null;
      pluginMessage=result.imported.length?`${result.imported.length}개를 가져왔습니다. 내용을 확인한 뒤 활성화하세요.`:'가져올 새 Claude 플러그인이 없습니다.';
      if(result.skipped.length) pluginMessage+=' '+result.skipped.map(d=>`${d.source}: ${d.message}`).join(' / ');
    } catch(e) {if(key===pluginKey()) pluginFailure=e.message;}
    finally {setBusy(false);renderPlugins();}
  });
  $("pluginSearch").addEventListener('input',()=>{pluginState.query=$("pluginSearch").value;renderPlugins();});
  $("pluginFilter").addEventListener('change',renderPlugins);
  $("pluginFolder").addEventListener('change',async()=>{
    if(busy||pluginLoading) return;
    const key=pluginKey(); setBusy(true); pluginFailure="";
    try {
      const picked=Array.from($("pluginFolder").files);
      if(!picked.length) return;
      const body=await collectPluginFiles(picked,$("pluginScope").value);
      if(key!==pluginKey()) return;
      const catalog=await api('/v1/plugins/import','POST',body);
      if(key!==pluginKey()) return;
      pluginState.receive(key,catalog); pluginState.selected=null;
      pluginMessage='가져왔습니다. 내용을 확인한 뒤 활성화하세요.';
    } catch(e) {if(key===pluginKey()) pluginFailure=e.message;}
    finally {$("pluginFolder").value="";setBusy(false);renderPlugins();}
  });
  function renderMcpConnections() {
    const key=pluginKey(), same=mcpLoaded;
    $("pluginMcpSave").disabled=busy||!same;
    $("pluginMcpLoad").disabled=busy;
    $("pluginMcpJson").disabled=busy;
    const panel=$("pluginMcpConnections"); panel.replaceChildren();
    if (!same) return;
    const pluginServers=(pluginState.scope===key?pluginState.catalog?.plugins||[]:[]).filter(p=>p.enabled).flatMap(p=>p.manifest.mcp_servers);
    for (const server of [...mcpServers,...pluginServers]) {
      const card=document.createElement('article');card.className='plugin-card';
      const title=document.createElement('h3');title.textContent=server.name;card.append(title);
      const status=document.createElement('p');status.textContent=server.oauth?'인증 상태를 확인하세요':'연결은 실행 시 확인됩니다.';card.append(status);
      if (server.oauth) {
        for(const [command,label] of [['status','상태 확인'],['login','로그인'],['cancel','로그인 취소'],['logout','로그아웃']]) button(card,label,async()=>{
          if(busy) return;
          setBusy(true);
          try {
            const result=await api(`/v1/mcp/${scope().provider}/${encodeURIComponent(server.name)}/oauth`,'POST',command);
            if(key!==pluginKey()) return;
            const labels={connected:'연결됨',signed_out:'로그인 필요',pending:'브라우저 승인 대기',expired:'인증 만료',cancelled:'로그인 취소됨',failed:'로그인 실패'};
            status.textContent=labels[result.state]||result.state;
            card.querySelector('a')?.remove();
            if(result.authorization_url) {
              const link=document.createElement('a'), url=new URL(result.authorization_url);
              if(url.protocol==='https:'||(['127.0.0.1','localhost','[::1]'].includes(url.hostname)&&url.protocol==='http:')) {
                link.href=url.href;link.textContent='로그인 페이지 열기';link.target='_blank';link.rel='noopener noreferrer';card.append(link);
              }
            }
          } finally {setBusy(false);}
        }).disabled=busy;
      }
      panel.append(card);
    }
    if(!panel.childElementCount) panel.textContent='등록된 MCP 연결이 없습니다.';
  }
  async function loadMcp() {
    if(busy) return;
    if(mcpDirty&&!confirm('저장하지 않은 MCP 변경을 버리고 다시 불러올까요?')) return;
    const key=pluginKey();setBusy(true);
    try {
      const servers=await api('/v1/mcp');
      if(key!==pluginKey()) return;
      mcpServers=servers;mcpLoaded=true;mcpDirty=false;
      $("pluginMcpJson").value=JSON.stringify(servers,null,2);
      $("pluginMcpStatus").textContent='공통 연결을 불러왔습니다. OAuth 인증은 제공자·계정별로 보관됩니다.';
    } finally {setBusy(false);renderMcpConnections();}
  }
  bind('pluginMcpLoad',loadMcp);
  $("pluginMcpJson").addEventListener('input',()=>{mcpDirty=true;$("pluginMcpStatus").textContent='저장하지 않은 변경이 있습니다.';});
  bind('pluginMcpSave',async()=>{
    if(busy||!mcpLoaded) return;
    const servers=parseJson($("pluginMcpJson").value), key=pluginKey();
    if(!Array.isArray(servers)) throw new Error('MCP JSON은 서버 배열이어야 합니다.');
    setBusy(true);
    try {
      const saved=await api('/v1/mcp','PUT',servers);
      if(key!==pluginKey()) return;
      mcpServers=saved;mcpDirty=false;$("pluginMcpStatus").textContent='저장되었습니다. 다음 턴부터 적용됩니다.';
    } finally {setBusy(false);renderMcpConnections();}
  });
  const codexPolicies=new Map();
  function renderCodexPolicy(){
    $("codexPolicy").hidden=scope().provider!=="codex";
    const policy=codexPolicies.get(JSON.stringify(scope()));
    $("approvalPolicy").value=policy?.approval_policy||"untrusted";
    $("sandbox").value=policy?.sandbox||"read-only";
  }
  for(const id of ["approvalPolicy","sandbox"]) $(id).addEventListener("change",()=>{
    codexPolicies.set(JSON.stringify(scope()),codexExecutionPolicy(scope().provider,$("approvalPolicy").value,$("sandbox").value));
  });
  let proposalTimer = null;
  const proposalCards = new Map();
  let proposalRefresh = 0;
  async function refreshSkillProposals() {
    if (!token) return;
    const credential = token;
    const generation = ++proposalRefresh;
    const [skills, servers] = await Promise.all([api("/v1/skill-proposals"), api("/v1/mcp-proposals")]);
    if (token !== credential || generation !== proposalRefresh) return;
    const proposals = [...skills, ...servers.map((p) => ({ ...p, kind: "mcp" }))];
    const live = new Set(proposals.map((p) => p.id));
    for (const [id, card] of proposalCards) {
      if (!live.has(id)) { card.remove(); proposalCards.delete(id); }
    }
    for (const p of proposals) {
      if (proposalCards.has(p.id)) continue;
      const card = document.createElement("div");
      const mcp = p.kind === "mcp";
      const heading = document.createElement("h3");
      heading.textContent = mcp ? `MCP 서버 제안 · ${p.server.name}` : `스킬 제안 · ${p.name}`;
      const hint = document.createElement("p");
      hint.textContent = `에이전트가 제안한 ${mcp ? "MCP 서버" : "스킬"}입니다. 승인하기 전에는 설치되지 않습니다${mcp ? "(승인하면 활성화되고 다음 턴부터 시작됩니다)" : ""} · 만료: ${new Date(p.expires_at * 1000).toLocaleTimeString()}`;
      const asked = document.createElement("p");
      asked.textContent = proposalRequestLine(p);
      card.append(heading, hint, asked);
      if (mcp) {
        const facts = document.createElement("pre");
        facts.textContent = mcpProposalLines(p).join("\n");
        card.append(facts);
      }
      for (const file of mcp ? [] : skillProposalFiles(p)) {
        const details = document.createElement("details");
        details.open = file.path.endsWith("/SKILL.md");
        const summary = document.createElement("summary");
        summary.textContent = file.path;
        const body = document.createElement("pre");
        body.textContent = file.text;
        details.append(summary, body);
        card.append(details);
      }
      const decide = async (allow) => {
        error(null);
        try { await api(`/v1/${mcp ? "mcp" : "skill"}-proposals/${p.id}`, "POST", { allow }); }
        finally { await refreshSkillProposals(); }
        if (allow) await (mcp ? refreshPlugins() : refreshGuidanceFiles().then(refreshGuidance));
      };
      button(card, "승인하고 설치", () => decide(true));
      button(card, "거절", () => decide(false));
      proposalCards.set(p.id, card);
      $("skillProposals").append(card);
    }
  }
  bind("skillProposalsRefresh", refreshSkillProposals);
  function error(e) {
    $("error").textContent = e ? String(e.message || e) : "";
  }
  function bind(id, action) {
    $(id).addEventListener("click", () => {
      error(null);
      Promise.resolve().then(action).catch(error);
    });
  }
  async function request(
    path,
    method = "GET",
    body,
    selected = scope(),
    signal,
  ) {
    const headers = {
      Authorization: `Bearer ${token}`,
      "X-KND-Web": "1",
      "X-KND-Account": selected.account || "default",
      "X-KND-Provider": selected.provider || "claude",
    };
    if (selected.workspace) headers["X-KND-Workspace"] = selected.workspace;
    if (body !== undefined) headers["Content-Type"] = "application/json";
    const response = await fetch(path, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      credentials: "omit",
      cache: "no-store",
      redirect: "error",
      signal,
    });
    if (!response.ok) {
      let detail;
      try {
        detail = await response.json();
      } catch {
        /* HTTP status is still useful. */
      }
      throw new Error(detail?.error || `HTTP ${response.status}`);
    }
    return response;
  }
  async function api(path, method, body) {
    const r = await request(path, method, body);
    return r.status === 204 || (r.status === 202 && method === "DELETE")
      ? null
      : parseJson(await r.text());
  }
  function options(id, values, selected) {
    $(id).replaceChildren(
      ...values.map(([value, label]) => {
        const o = document.createElement("option");
        o.value = value;
        o.textContent = label;
        return o;
      }),
    );
    if (selected !== undefined) $(id).value = selected;
  }
  function button(parent, text, action) {
    const b = document.createElement("button");
    b.textContent = text;
    b.addEventListener("click", () =>
      Promise.resolve().then(action).catch(error),
    );
    parent.append(b);
    return b;
  }
  function setBusy(value) {
    busy = value;
    for (const id of [
      "provider",
      "account",
      "workspace",
      "model",
      "approvalPolicy",
      "sandbox",
      "send",
      "new",
      "find",
      "login",
      "logout",
      "rename",
      "delete",
      "verificationCommands",
      "verificationSave",
    ])
      $(id).disabled = value;
    $("cancel").disabled = !value;
    renderGuidance();
    renderGuidanceEditor();
    renderPlugins();
    $("pluginMcpLoad").disabled=value;
    $("pluginMcpSave").disabled=value||!mcpLoaded;
    $("pluginMcpJson").disabled=value;
    $("pluginMcpConnections").querySelectorAll('button').forEach(button=>button.disabled=value);
    renderGit();
    document
      .querySelectorAll("#history button")
      .forEach((b) => (b.disabled = value));
  }
  function clear() {
    guidanceSelection.selected=[];
    renderGuidance();
    conversation = null;
    attachments = [];
    $("messages").replaceChildren();
    jumpLatest.hidden = true;
    $("timeline").replaceChildren();
    $("notices").replaceChildren();
    $("interactions").replaceChildren();
    workCollapsed = false;
    renderWorkPanel();
    renderAttachments();
  }
  function message(text, user) {
    const follow = followsTranscriptTail(transcript);
    const e = document.createElement("div");
    e.className = "message" + (user ? " user" : "");
    e.textContent = text;
    $("messages").append(e);
    if (follow) transcript.scrollTop = transcript.scrollHeight;
    updateJump();
    return e;
  }
  function image(attachment) {
    const e = document.createElement("img");
    e.alt = attachment.name;
    e.src = `data:${attachment.mime_type};base64,${attachment.data}`;
    return e;
  }
  function renderAttachments() {
    $("attachments").replaceChildren();
    attachments.forEach((a, i) => {
      const row = document.createElement("div");
      row.textContent = a.name;
      if (a.type === "image") row.append(image(a));
      button(row, "첨부 제거", () => {
        attachments.splice(i, 1);
        renderAttachments();
      });
      $("attachments").append(row);
    });
  }
  function addAttachment(a) {
    const next = [...attachments, a];
    const bytes = (a) =>
      a.type === "image"
        ? Math.ceil(a.data.length * 0.75)
        : new TextEncoder().encode(a.text).length;
    if (
      next.length > 8 ||
      next.reduce((n, a) => n + bytes(a), 0) > 4 * 1024 * 1024
    )
      throw new Error("첨부는 최대 8개 / 합계 4 MiB입니다");
    attachments = next;
    renderAttachments();
  }
  async function history() {
    const rows = await api(
      `${endpoint()}/conversations?q=${encodeURIComponent($("search").value)}`,
    );
    $("history").replaceChildren();
    for (const row of rows) {
      const b = button(
        $("history"),
        row.title +
          (row.matched_message ? `\n${row.matched_message.preview}` : ""),
        async () => {
          if (!busy) {
            setBusy(true);
            try {
              await load(row.id, row.matched_message?.message_id);
            } finally {
              setBusy(false);
            }
          }
        },
      );
      b.disabled = busy;
    }
  }
  async function load(id, focus, preserveDraft = false) {
    const c = await api(`${endpoint()}/conversations/${id}`);
    const previous = preserveDraft ? {scrollTop:transcript.scrollTop,scrollHeight:transcript.scrollHeight,clientHeight:transcript.clientHeight} : null;
    const draft = attachments;
    const collapsed = workCollapsed;
    clear();
    if (preserveDraft) workCollapsed = collapsed;
    if (preserveDraft) {
      attachments = draft;
      renderAttachments();
    }
    conversation = c.id;
    if (c.model) $("model").value = c.model;
    for (const m of c.messages) {
      const e = message(m.text + (m.incomplete ? "\n[미완료]" : ""), m.user);
      if(m.guidance && m.skills?.length) { const applied=document.createElement("small"); applied.textContent=`이 턴에 적용된 스킬: ${m.skills.map(skill=>skill.id).join(", ")}`; e.append(applied); }
      e.id = `message-${m.id}`;
      for (const a of m.attachments)
        if (a.type === "image") {
          const img = image(a);
          img.width = 180;
          e.append(img);
        }
      const emit = handler(null, e);
      for (const entry of c.timeline.filter((entry) => entry.message_id === m.id)) emit(entry.event);
    }
    renderWorkPanel();
    await Promise.all(
      [...$("messages").querySelectorAll("img")].map((img) =>
        img.decode().catch(() => {}),
      ),
    );
    if (focus)
      document
        .getElementById(`message-${focus}`)
        ?.scrollIntoView({ block: "center" });
    else transcript.scrollTop = restoredTranscriptTop(transcript, previous);
    updateJump();
  }
  async function selected() {
    renderCodexPolicy();
    clear();
    await refreshGit();
    pluginState.begin(pluginKey());
    pluginMessage="";pluginFailure="";pluginLoading=false;
    if($("pluginSettings").open) await refreshPlugins();
    if(!mcpLoaded) $("pluginMcpStatus").textContent='공통 연결을 불러오세요.';
    renderMcpConnections();
    guidanceSelection.begin(JSON.stringify(scope()));
    await refreshGuidance();
    renderGuidanceEditor();
    if ($("guidanceSettings").open || $("skillsSettings").open) await refreshGuidanceFiles();
    $("status").textContent = "확인 중…";
    const info = infos.find((i) => i.provider === scope().provider);
    $("support").textContent =
      `캡처는 이미지 첨부로 전달합니다. ${info?.capabilities.image_capability_check ? "선택한 ACP 런타임·모델의 이미지 지원 확인이 필요합니다." : "Claude·Codex는 이미지 첨부를 지원합니다."} 사용자 질문: ${info?.capabilities.user_input ? "지원" : "런타임 미지원"}`;
    try {
      const status = await api(`${endpoint()}/status`);
      $("status").textContent = status.authenticated
        ? `로그인됨 ${status.account || ""}`
        : "로그인 필요 (공개 모델 제외)";
    } catch (e) {
      $("status").textContent = info?.issue || e.message;
    }
    try {
      const models = await api(`${endpoint()}/models`);
      options("model", [
        ["", "기본 모델"],
        ...models.map((m) => [m.id, m.name]),
      ]);
    } catch (e) {
      options("model", [["", "기본 모델"]]);
      error(e);
    }
    await history();
  }
  async function providerChanged() {
    const accounts = await api(`${endpoint()}/accounts`);
    options(
      "account",
      accounts.map((a) => [a.id, a.label]),
    );
    await selected();
  }
  async function connect() {
    token = $("token").value.trim();
    $("token").value = "";
    infos = await api("/v1/providers");
    options(
      "provider",
      infos.map((i) => [i.provider, i.name]),
    );
    const catalog = await api("/v1/projects");
    projectCatalog = catalog;
    options("workspace", [
      ["", "서버 기본 작업공간"],
      ...catalog.projects.flatMap((p) =>
        p.workspaces
          .filter((w) => !w.missing)
          .map((w) => [String(w.id), `${p.name} · ${w.branch || w.path}`]),
      ),
    ]);
    renderVerificationSettings();
    $("connect").hidden = true;
    $("app").hidden = false;
    setBusy(true);
    try {
      await providerChanged();
      await controlStatus();
      await refreshSkillProposals();
      clearInterval(proposalTimer);
      proposalTimer = setInterval(() => { refreshSkillProposals().catch(error); }, 5000);
    } finally {
      setBusy(false);
    }
  }
  bind("connectButton", connect);
  bind("disconnect", () => {
    clearInterval(proposalTimer);
    proposalRefresh++;
    $("skillProposals").replaceChildren();
    proposalCards.clear();
    controller?.abort();
    token = "";
    gitRequest++;gitLoading=false;gitRestore=null;gitFailure=false;gitReview.begin(null);$("gitStatus").textContent="";
    job = operation = null;
    clear();
    frame = null;
    $("frame").src = "";
    $("frame").hidden = true;
    $("app").hidden = true;
    $("connect").hidden = false;
    setBusy(false);
    error(null);
  });
  $("provider").addEventListener("change", async () => {
    setBusy(true);
    try {
      await providerChanged();
    } catch (e) {
      error(e);
    } finally {
      setBusy(false);
    }
  });
  for (const id of ["account", "workspace"])
    $(id).addEventListener("change", async () => {
      setBusy(true);
      try {
        renderVerificationSettings();
        await selected();
      } catch (e) {
        error(e);
      } finally {
        setBusy(false);
      }
    });
  bind("find", history);
  bind("new", clear);
  bind("rename", async () => {
    if (!conversation) return;
    const title = prompt("새 대화 제목");
    if (title?.trim()) {
      await api(`${endpoint()}/conversations/${conversation}`, "PATCH", {
        title,
      });
      await history();
    }
  });
  bind("delete", async () => {
    if (conversation && confirm("이 대화와 메시지를 영구 삭제할까요?")) {
      await api(`${endpoint()}/conversations/${conversation}`, "DELETE");
      clear();
      await history();
    }
  });
  bind("logout", async () => {
    await api(`${endpoint()}/logout`, "POST");
    await selected();
  });
  async function lines(response, emit) {
    const reader = response.body.getReader(),
      decoder = new TextDecoder();
    let buffer = "";
    try {
      while (true) {
        const { done, value } = await reader.read();
        buffer += decoder.decode(value, { stream: !done });
        if (buffer.length > 8 * 1024 * 1024)
          throw new Error("스트림 이벤트가 너무 큽니다");
        let at;
        while ((at = buffer.indexOf("\n")) >= 0) {
          const line = buffer.slice(0, at);
          buffer = buffer.slice(at + 1);
          if (line.trim()) emit(parseJson(line));
        }
        if (done) break;
      }
      if (buffer.trim()) emit(parseJson(buffer));
    } finally {
      await reader.cancel().catch(() => {});
    }
  }
  function interaction(event, base) {
    const panel = document.createElement("section"),
      pre = document.createElement("pre");
    pre.textContent = JSON.stringify(event.event, null, 2);
    if (event.event.type !== "user_input") panel.append(pre);
    $("interactions").append(panel);
    async function respond(body) {
      await api(`${base}/interactions/${event.request_id}`, "POST", body);
      error(null);
      panel.remove();
    }
    if (event.event.type === "elicitation") {
      let input;
      if (event.event.schema) {
        const label = document.createElement("label");
        label.textContent = "MCP 입력 (JSON)";
        input = document.createElement("textarea");
        input.rows = 5;
        input.maxLength = 65536;
        input.placeholder = "요청한 형식에 맞는 JSON을 입력하세요.";
        label.append(input);
        panel.append(label);
      }
      if (event.event.url) {
        const url = new URL(event.event.url);
        if (["https:", "http:"].includes(url.protocol) && !url.username && !url.password) {
          const link = document.createElement("a");
          link.href = url.href;
          link.textContent = `요청 페이지 열기: ${url.href}`;
          link.target = "_blank";
          link.rel = "noreferrer noopener";
          panel.append(link);
        }
      }
      button(panel, "수락", () => respond({
        type: "elicitation", action: "accept",
        content: input ? parseJson(input.value) : null,
      }));
      button(panel, "거절", () => respond({ type: "elicitation", action: "decline", content: null }));
    } else if (event.event.type === "user_input") {
      let send;
      function answers() {
        return Object.fromEntries(fields.map(({q, choices, input}) => [q.id,
          questionAnswer(q.multi_select, choices.filter(c=>c.checked).map(c=>c.value), input.value)]));
      }
      function refresh() {
        try { answers(); send.disabled = false; }
        catch { send.disabled = true; }
      }
      const fields = event.event.questions.map((q) => {
        const group = document.createElement("fieldset"), legend = document.createElement("legend");
        legend.textContent = q.question;
        group.append(legend);
        const choices = q.options.map((option) => {
          const label = document.createElement("label"), choice = document.createElement("input");
          label.className = "question-option";
          choice.type = q.multi_select ? "checkbox" : "radio";
          choice.name = `question-${event.request_id}-${q.id}`;
          choice.value = option;
          choice.addEventListener("change", () => {
            if (!q.multi_select && choice.checked) input.value = "";
            refresh();
          });
          label.append(choice, document.createTextNode(option));
          group.append(label);
          return choice;
        });
        const label = document.createElement("label");
        label.textContent = "직접 입력 (선택)";
        const input = document.createElement("input");
        input.maxLength = 4096;
        input.addEventListener("input", () => {
          if (!q.multi_select && input.value.trim()) choices.forEach(choice=>choice.checked=false);
          refresh();
        });
        label.append(input);
        group.append(label);
        panel.append(group);
        return {q, choices, input};
      });
      send = button(panel, "답변 전송", () =>
        respond({
          type: "answers",
          answers: answers(),
        }),
      );
      refresh();
    } else if (event.event.type === "permission") {
      // ACP requires an explicit allow_once option; never substitute allow_always.
      const allow =
        event.event.method !== "session/request_permission" ||
        event.event.details?.options?.some((o) => o.kind === "allow_once");
      if (allow)
        button(panel, event.event.method === "item/permissions/requestApproval" ? "현재 턴 허용" : "이번 요청 승인", () =>
          respond({ type: "approval", allow: true }),
        );
      button(panel, "거절", () => respond({ type: "approval", allow: false }));
    }
    button(panel, "질문 취소", () => respond({ type: "cancelled" }));
  }
  function handler(base, reply) {
    const tools = new Map();
    const verifications = new Map();
    function renderVerification(report) {
      const key = `${report.job_id}-${report.attempt}`;
      let run = verifications.get(key);
      if (!run) {
        run = {card:document.createElement("section"),heading:document.createElement("h3"),version:document.createElement("small"),checks:[]};
        run.card.className = "verification";
        run.card.append(run.heading,run.version);
        verifications.set(key,run);
        $("timeline").append(run.card);
      }
      const changed = report.finished_revision && (report.revision.head !== report.finished_revision.head || report.revision.fingerprint !== report.finished_revision.fingerprint);
      const label = verificationPassed(report) ? "필수 검증 통과" : report.finished_revision || report.checks.some(check=>check.status==="failed") ? "필수 검증 실패" : "필수 검증 중";
      run.heading.textContent = `${label} · 시도 ${report.attempt}${changed ? " · 코드 변경으로 재검사 필요" : ""}`;
      run.version.textContent = `HEAD: ${report.revision.head || "unborn / plain folder"}\nSHA-256: ${report.revision.fingerprint}`;
      for (const [index,check] of report.checks.entries()) {
        let row = run.checks[index];
        if (!row) {
          row = {details:document.createElement("details"),summary:document.createElement("summary"),output:document.createElement("pre")};
          row.details.append(row.summary,row.output);
          run.card.append(row.details);
          run.checks[index] = row;
        }
        const status = {pending:"대기",running:"실행 중",passed:"통과",failed:"실패"}[check.status];
        row.summary.textContent = `${status} · ${check.exit_code ?? "—"} · ${check.command.join(" ")}`;
        row.output.textContent = `${JSON.stringify(check.command,null,2)}\n종료 코드: ${check.exit_code ?? "없음"}\n${check.output}`;
      }
    }
    function renderTool(t) {
      t.summary.textContent = `${t.call.status || "pending"} · ${t.call.title || t.call.id}`;
      t.pre.textContent = JSON.stringify(t.call, null, 2);
    }
    return (event) => {
      if (event.type === "started") operation = event.operation_id;
      if (event.type === "verification") renderVerification(event.report);
      if (event.type === "conversation") conversation = event.conversation_id;
      if (event.type === "text") {
        updateReply(reply, reply.textContent + event.delta);
      }
      if (event.type === "completed") updateReply(reply, event.text);
      if (event.type === "runtime_status") {
        const status = document.createElement("p");
        status.textContent = `${event.status} · ${event.message}`;
        status.setAttribute("role", isWorkEvent(event) ? "status" : "alert");
        $(isWorkEvent(event) ? "timeline" : "notices").append(status);
      }
      if (event.type === "input_delivery") {
        const delivery = document.createElement("p");
        delivery.textContent = `#${event.message_id} · ${event.delivered ? "런타임 접수" : "수신함 유지"}`;
        $("timeline").append(delivery);
      }
      if (event.type === "error") error(event.message);
      if (event.type === "cancelled") updateReply(reply, reply.textContent + "\n[취소됨]");
      if (event.type === "interaction")
        interaction(event, base || `/v1/operations/${event.operation_id}`);
      if (event.type === "authorization") {
        const url = new URL(event.url);
        if (["https:", "http:"].includes(url.protocol)) {
          const a = document.createElement("a");
          a.href = url.href;
          a.textContent = `로그인 페이지 열기 ${event.code || ""}`;
          a.target = "_blank";
          a.rel = "noreferrer noopener";
          $("interactions").append(a);
        }
      }
      if (event.type === "tool") {
        let t = tools.get(event.call.id);
        if (!t) {
          t = { call: {}, details: document.createElement("details"), summary: document.createElement("summary"), pre: document.createElement("pre") };
          t.details.append(t.summary, t.pre);
          tools.set(event.call.id, t);
          $("timeline").append(t.details);
        }
        mergeToolCall(t.call, event.call);
        renderTool(t);
        if (t.call.status === "failed" && !t.failure) {
          t.failure = document.createElement("p");
          t.failure.setAttribute("role", "alert");
          t.failure.textContent = `도구 실행 실패 · ${t.call.title || t.call.id}`;
          $("notices").append(t.failure);
        }
      }
      if (["completed", "cancelled", "error", "turn_end"].includes(event.type)) {
        for (const t of tools.values()) {
          finishToolCall(t.call);
          renderTool(t);
        }
      }
      if (event.type === "interaction_resolved") {
        $("interactions").replaceChildren();
        const confirmation = document.createElement("p");
        const response = event.response;
        confirmation.textContent = "사용자 확인 · " + (response.type === "approval" ? (response.allow ? "승인함" : "거절함") : response.type === "cancelled" ? "취소함" : "응답 전달됨");
        $("notices").append(confirmation);
      }
      if (isWorkEvent(event)) renderWorkPanel();
    };
  }
  bind("login", async () => {
    setBusy(true);
    operation = null;
    controller = new AbortController();
    try {
      const response = await request(
        `${endpoint()}/login`,
        "POST",
        { device_code: false },
        scope(),
        controller.signal,
      );
      await lines(response, handler(null, message("로그인 진행 중…", false)));
    } finally {
      operation = null;
      controller = null;
      try {
        if (token) await selected();
      } finally {
        setBusy(false);
      }
    }
  });
  async function watchJob(id) {
    job = id;
    operation = null;
    setBusy(true);
    controller = new AbortController();
    const reply = message("", false);
    $("timeline").replaceChildren();
    $("interactions").replaceChildren();
    renderWorkPanel();
    try {
      const response = await request(
        `/v1/jobs/${id}/events`,
        "GET",
        undefined,
        scope(),
        controller.signal,
      );
      const emit = handler(`/v1/jobs/${id}`, reply);
      await lines(response, (e) => emit(e.event));
      const detail = await api(`/v1/jobs/${id}`);
      if (detail.job.error) error(detail.job.error);
      if (detail.job.conversation_id)
        await load(detail.job.conversation_id, undefined, true);
    } finally {
      job = operation = null;
      controller = null;
      setBusy(false);
      renderWorkPanel();
      if (token) await history();
      if (token) await refreshGit();
    }
  }
  bind("send", async () => {
    if (busy || (!$("prompt").value.trim() && !attachments.length)) return;
    const body = {
      skills:guidanceSelection.selected.slice(),
      text: $("prompt").value,
      model: $("model").value || null,
      conversation_id: conversation,
      session_id: null,
      attachments,
      codex_execution_policy:codexExecutionPolicy(scope().provider,$("approvalPolicy").value,$("sandbox").value),
    };
    setBusy(true);
    try {
      const queued = await api(`${endpoint()}/jobs`, "POST", body);
      guidanceSelection.selected=[];
      renderGuidance();
      message(body.text, true);
      if ($("prompt").value === body.text) $("prompt").value = "";
      attachments = attachments.filter((a) => !body.attachments.includes(a));
      renderAttachments();
      await watchJob(queued.id);
    } finally {
      setBusy(false);
    }
  });
  bind("cancel", async () => {
    if (job) await api(`/v1/jobs/${job}`, "DELETE");
    else if (operation) await api(`/v1/operations/${operation}`, "DELETE");
  });
  $("files").addEventListener("change", async () => {
    try {
      for (const f of $("files").files) {
        if (f.size > 2 * 1024 * 1024)
          throw new Error("파일이 2 MiB를 넘습니다");
        if (["image/png", "image/jpeg", "image/webp"].includes(f.type)) {
          const data = await new Promise((resolve, reject) => {
            const r = new FileReader();
            r.onload = () => resolve(r.result.split(",")[1]);
            r.onerror = reject;
            r.readAsDataURL(f);
          });
          addAttachment({
            type: "image",
            name: f.name,
            mime_type: f.type,
            data,
          });
        } else {
          if (f.size > 256 * 1024)
            throw new Error("텍스트는 256 KiB까지 가능합니다");
          const text = new TextDecoder("utf-8", { fatal: true }).decode(
            await f.arrayBuffer(),
          );
          addAttachment({ type: "text", name: f.name, text });
        }
      }
    } catch (e) {
      error(e);
    } finally {
      $("files").value = "";
    }
  });
  async function controlStatus() {
    const s = await api("/v1/control");
    const computer = await api("/v1/control/computer");
    $("grant").disabled = computer.automatic;
    $("controlStatus").textContent =
      `브라우저: ${s.browser_available ? (s.browser_running ? "실행 중" : "사용 가능") : "설치 필요"} / Windows: ${s.desktop_available ? (computer.automatic ? "호스트 실행 중 자동 허용" : s.desktop_enabled_until * 1000 > Date.now() ? "제어 허용됨" : "꺼짐") : "미지원"}`;
  }
  let controlBusy = false;
  async function control(command) {
    if (controlBusy) throw new Error("제어 요청이 진행 중입니다");
    controlBusy = true;
    const controls = [
      "open",
      "reload",
      "capture",
      "type",
      "press",
      "up",
      "down",
      "target",
    ];
    controls.forEach((id) => ($(id).disabled = true));
    try {
      const next = await api(
        `/v1/control/${$("target").value}`,
        "POST",
        command,
      );
      frame = next;
      $("frame").hidden = !frame;
      if (frame)
        $("frame").src =
          `data:${frame.image.mime_type};base64,${frame.image.data}`;
      await controlStatus();
    } finally {
      controlBusy = false;
      controls.forEach((id) => ($(id).disabled = false));
    }
  }
  options(
    "key",
    [
      "Enter",
      "Tab",
      "Escape",
      "Backspace",
      "ArrowUp",
      "ArrowDown",
      "ArrowLeft",
      "ArrowRight",
      "Home",
      "End",
      "Delete",
    ].map((k) => [k, k]),
  );
  bind("controlRefresh", controlStatus);
  bind("open", () =>
    control({ type: "open", url: $("url").value, mobile: $("mobile").checked }),
  );
  bind("reload", () => control({ type: "reload" }));
  bind("capture", () => control({ type: "capture" }));
  bind("grant", async () => {
    if (
      confirm(
        "서버의 Windows 전체 화면 캡처와 마우스·키보드 제어를 10분 동안 허용할까요?",
      )
    ) {
      await api("/v1/control/desktop/grant", "POST");
      await controlStatus();
    }
  });
  bind("stopControl", async () => {
    await api(`/v1/control/${$("target").value}`, "DELETE");
    frame = null;
    $("frame").hidden = true;
    $("frame").src = "";
    await controlStatus();
  });
  bind("attachCapture", () => {
    if (!frame) throw new Error("먼저 화면을 캡처하세요");
    addAttachment(frame.image);
  });
  bind("type", () => control({ type: "text", text: $("inputText").value }));
  bind("press", () => control({ type: "key", key: $("key").value }));
  bind("up", () => control({ type: "scroll", delta: -480 }));
  bind("down", () => control({ type: "scroll", delta: 480 }));
  $("target").addEventListener("change", () => {
    $("browserControls").hidden = $("target").value === "desktop";
    $("grant").hidden = $("target").value !== "desktop";
    frame = null;
    $("frame").hidden = true;
    $("frame").src = "";
    $("clickControl").checked = false;
  });
  $("frame").addEventListener("click", (e) => {
    if (!frame || !$("clickControl").checked) return;
    const r = e.target.getBoundingClientRect();
    const scale = Math.min(
      r.width / e.target.naturalWidth,
      r.height / e.target.naturalHeight,
    );
    const w = e.target.naturalWidth * scale,
      h = e.target.naturalHeight * scale;
    const x = e.clientX - r.left - (r.width - w) / 2,
      y = e.clientY - r.top - (r.height - h) / 2;
    if (x >= 0 && y >= 0 && x < w && y < h)
      control({
        type: "click",
        x: Math.floor((x / w) * frame.width),
        y: Math.floor((y / h) * frame.height),
      }).catch(error);
  });
  if ("serviceWorker" in navigator && window.isSecureContext)
    navigator.serviceWorker.register("/sw.js").catch(error);
}
