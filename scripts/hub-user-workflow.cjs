/* Real production SDK submission + HTTP SSE, without Electron or a desktop. */
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const vm = require('node:vm');
const crypto = require('node:crypto');
const ts = require(require.resolve('typescript', {paths:[path.resolve(__dirname,'../../deepcloud-ui')]}));
const { spawnSync } = require('node:child_process');
const hash = data => crypto.createHash('sha256').update(data).digest('hex');
const args = process.argv.slice(2);
const option = name => args[args.indexOf(name) + 1];
if (args.includes('--redaction-self-test')) {
  const assert = require('node:assert/strict');
  const original = { output: JSON.stringify({ code: 'const x="https://example.invalid/a?sig=private";\nconst y="quoted";' }),
    nested: [{ apiKey: 'dummy-private-value', url: 'https://example.invalid/a?token=private' }] };
  const result = JSON.parse(JSON.stringify(redact(original)));
  assert.equal(typeof result.output, 'string');
  assert.ok(!result.output.includes('sig=private'));
  assert.equal(result.nested[0].apiKey, '[REDACTED]');
  assert.ok(result.nested[0].url.endsWith('/[REDACTED_REMOTE_URL]'));
  console.log('nested JSON and signed URL redaction passed');
  process.exit(0);
}
const output = path.resolve(option('--output'));
if (fs.existsSync(output)) throw Error('Never overwrite a previous attempt');
const sdkPath = path.resolve(__dirname, '../../deepcloud-ui/src/lib/agent-sdk/client.ts');
const sdkSource = fs.readFileSync(sdkPath, 'utf8');
const context = { exports: {}, console, process, fetch, AbortController,
  require: id => { if (id === '../api') return { getLocalGatewayHeaders: () => ({}) }; throw Error(`unexpected SDK runtime import: ${id}`); } };
vm.runInNewContext(ts.transpileModule(sdkSource, { compilerOptions: { module: ts.ModuleKind.CommonJS } }).outputText, context);
const client = new context.exports.SuperAgentClient({ baseUrl: args.includes('--base') ? option('--base') : 'http://127.0.0.1:3001' });
const report = { schema_version: 1, sdk: { path: sdkPath, sha256: hash(sdkSource) },
  transport: 'production SDK runAgent + actual HTTP SSE', base: client.getBaseUrl(),
  model: 'gemini-3.8-flash-high', classification: 'sparse_real_user_workflow', desktop: false, attempts: [] };
if (args.includes('--identity')) {
  const identityPath = path.resolve(option('--identity'));
  const identity = JSON.parse(fs.readFileSync(identityPath, 'utf8').replace(/^\uFEFF/, ''));
  if (String(new URL(client.getBaseUrl()).port) !== String(identity.port)) throw Error('Runtime identity port mismatch');
  const actual = hash(fs.readFileSync(path.join(identity.stage, 'hub.exe')));
  if (actual.toLowerCase() !== identity.binarySha256.toLowerCase()) throw Error('Runtime binary fingerprint mismatch');
  report.runtime = { ...identity, identity_path: identityPath, verified_binary_sha256: actual };
}
const persist = () => { fs.mkdirSync(path.dirname(output), { recursive: true }); const temporary=output+'.tmp'; fs.writeFileSync(temporary, JSON.stringify(report, null, 2)); fs.renameSync(temporary,output); };
// Redact signed URLs and any accidentally echoed credentials before persistence.
function redact(value) {
  if (typeof value === 'string') return value.replace(/sk-[a-zA-Z0-9_-]{16,}/g, '[REDACTED_KEY]')
    .replace(/https?:\/\/[^\s"'<>]+/g, url => {
      try {
        const parsed = new URL(url);
        if (["commons.wikimedia.org","www.mediawiki.org","upload.wikimedia.org","www.wikimedia.org"].includes(parsed.hostname)) { for(const k of parsed.searchParams.keys()) if(/token|password|secret|auth|signature|api.?key/i.test(k)) parsed.searchParams.set(k,"[REDACTED]"); return parsed.href; }
        if (!['127.0.0.1', 'localhost', '[::1]'].includes(parsed.hostname)) return `${parsed.protocol}//${parsed.hostname}/[REDACTED_REMOTE_URL]`;
        return url.includes('?') ? url.split('?')[0] + '?[REDACTED_QUERY]' : url;
      } catch { return '[REDACTED_URL]'; }
    });
  if (Array.isArray(value)) return value.map(redact);
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([key, item]) =>
    [key, /^(authorization|api[_-]?key|cookie|set-cookie|access[_-]?token)$/i.test(key) ? '[REDACTED]' : redact(item)]));
  return value;
}

const dialogs = path.resolve(option("--dialog-dir"));
fs.mkdirSync(dialogs,{recursive:true});
const pendingQuestions=new Map();let replying=false;
const questionKey=id=>hash(id).slice(0,20);
const replyPoll=setInterval(async()=>{if(replying)return;replying=true;try{for(const [id,q] of pendingQuestions){const key=questionKey(id),file=path.join(dialogs,key+".reply.json");if(!fs.existsSync(file))continue;let v;try{v=JSON.parse(fs.readFileSync(file,"utf8"))}catch{continue}if(v.request_id!==id||typeof v.response!=="string"||!v.response.trim())continue;pendingQuestions.delete(id);let result;try{result=await client.interact(q.task_id,{action:"respond",requestId:id,response:v.response})}catch(error){result={error:String(error)}}fs.writeFileSync(path.join(dialogs,key+".sent.json"),JSON.stringify({request_id:id,response:v.response,result},null,2));}}finally{replying=false}},250);replyPoll.unref();
async function execute(definition) {
  const started = Date.now();
  const attempt = { id: definition.id, project: definition.request.project, request: definition.request,
    events: [], status: 'submitted', classification: definition.classification || 'first_attempt', prior_exposure: definition.prior_exposure ?? null };
  report.attempts.push(attempt); persist();
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), ((definition.request.timeoutSeconds || 300) + 15) * 1000);
  try {
    const submitted = await client.runAgent(definition.request);
    attempt.task_id = submitted.taskId; persist();
    const response = await fetch(`${client.getBaseUrl()}/api/agents/tasks/${encodeURIComponent(submitted.taskId)}/events`, { signal: controller.signal });
    if (!response.ok) throw Error(`SSE ${response.status}`);
    let buffer = ''; const decoder = new TextDecoder();
    for await (const bytes of response.body) {
      buffer += decoder.decode(bytes, { stream: true });
      const lines = buffer.split('\n'); buffer = lines.pop();
      for (const line of lines) if (line.startsWith('data:')) {
        let event;
        try { event = JSON.parse(line.slice(5).trim()); } catch { continue; }
        attempt.events.push(redact(event));
        if(event.kind==="interaction-request"&&event.request_id&&!pendingQuestions.has(event.request_id)){const key=questionKey(event.request_id);pendingQuestions.set(event.request_id,{task_id:attempt.task_id});fs.writeFileSync(path.join(dialogs,key+".question.json"),JSON.stringify({task_id:attempt.task_id,event:redact(event),events_before_question:attempt.events.length},null,2));}

        if (event.kind === 'status' && ['completed', 'failed', 'cancelled', 'finished', 'timeout'].includes(event.status)) attempt.status = event.status;
        persist();
      }
    }
  } catch (error) {
    attempt.error = error.name + ': ' + error.message;
    if (!['completed', 'failed', 'cancelled', 'finished', 'timeout'].includes(attempt.status)) attempt.status = controller.signal.aborted ? 'timeout' : 'infra_failed';
    if (attempt.task_id && (controller.signal.aborted || !['completed', 'failed', 'cancelled', 'finished', 'timeout'].includes(attempt.status))) {
      await client.cancelTask(attempt.task_id).catch(() => {});
      attempt.cancellation_requested = true;
    }
  } finally {
    clearTimeout(timer);
    attempt.duration_ms = Date.now() - started;
    attempt.tool_calls = attempt.events.filter(e => e.kind === 'tool-use').length;
    attempt.model_turns = attempt.events.filter(e => e.kind === 'trace-snapshot').length;
    attempt.usage_events = attempt.events.filter(e => /usage|token/.test(e.kind || ''));
    // Absence is explicit; no invented token usage from event counts.
    attempt.token_usage = attempt.usage_events.length ? attempt.usage_events : null;
    if (attempt.task_id) {
      const task = await fetch(`${client.getBaseUrl()}/api/tasks/${encodeURIComponent(attempt.task_id)}`).then(r => r.json()).catch(() => null);
      if (task?.workspaceBaseline) attempt.execution_workspace = path.join(path.dirname(task.workspaceBaseline), 'workspace');
    }
    if (definition.oracle === 'readonly') {
      const file = definition.readonly_path || 'report.md';
      attempt.oracle_passed = fs.readFileSync(path.join(definition.request.project, file), 'utf8') === definition.original_report
        && (!args.includes('--isolated') || fs.readFileSync(path.join(attempt.execution_workspace, file), 'utf8') === definition.original_report)
        && !attempt.events.some(e => e.kind === 'tool-use' && ['write_to_file', 'replace_in_file', 'multi_replace_file_content'].includes(e.tool));
    } else if (definition.oracle === 'checkout') {
      const actualWorkspace = args.includes('--isolated') ? attempt.execution_workspace : definition.request.project;
      const oracle = spawnSync(process.execPath, ['--test', definition.test_path], { cwd: actualWorkspace, encoding: 'utf8' });
      attempt.oracle = { exit_code: oracle.status, stdout: oracle.stdout, stderr: oracle.stderr };
      attempt.oracle_passed = oracle.status === 0 && hash(fs.readFileSync(definition.test_path)) === definition.test_sha256;
      if (args.includes('--isolated')) {
        attempt.origin_unchanged = hash(fs.readFileSync(path.join(definition.request.project, 'calc.cjs'))) === definition.original_source_sha256;
        attempt.oracle_passed &&= attempt.origin_unchanged;
      }
    } else if (definition.oracle === 'dmghg' && attempt.execution_workspace) {
      const command = definition.oracle_command.map(arg => arg.replaceAll('{workspace}', attempt.execution_workspace));
      const oracle = spawnSync(command[0], command.slice(1), { cwd: attempt.execution_workspace, encoding: 'utf8', timeout: 200000 });
      try { attempt.oracle = JSON.parse(oracle.stdout.trim().split('\n').at(-1)); }
      catch { attempt.oracle = { passed: false, error: 'oracle did not emit a structured result' }; }
      attempt.oracle.exit_code = oracle.status;
      attempt.oracle_passed = oracle.status === 0 && attempt.oracle.passed === true;
      attempt.artifacts = ['rule.json', 'bridge.py', 'README.md'].filter(file => fs.existsSync(path.join(attempt.execution_workspace, file)))
        .map(file => ({ path: path.join(attempt.execution_workspace, file), sha256: hash(fs.readFileSync(path.join(attempt.execution_workspace, file))) }));
    }
    attempt.passed = definition.oracle ? attempt.status === 'completed' && attempt.oracle_passed === true : null;
    persist();
    console.log(JSON.stringify({ id: attempt.id, task: attempt.task_id, status: attempt.status, passed: attempt.passed, duration_ms: attempt.duration_ms, tools: attempt.tool_calls }));
  }
}

(async () => {
  const models = await client.listModels();
  report.configured_models = models;
  if (!models.includes(report.model)) throw Error('Requested model not configured');
  let definitions;
  if (args.includes('--requests')) {
    definitions = JSON.parse(fs.readFileSync(option('--requests'), 'utf8'));
    report.request_file_sha256 = hash(fs.readFileSync(option('--requests')));
  } else {
    const stage = fs.mkdtempSync(path.join(os.tmpdir(), 'hub-delivery-p0-'));
    report.stage = stage;
    const review = path.join(stage, 'review'); const checkout = path.join(stage, 'checkout');
    fs.mkdirSync(review); fs.mkdirSync(checkout);
    const text = '# 调查记录\n只验证了一个样本，未生成规则，手机未验证。审计标记：REVIEW_SCOPE_42。\n';
    fs.writeFileSync(path.join(review, 'report.md'), text);
    fs.writeFileSync(path.join(checkout, 'calc.cjs'), 'exports.shippingFee = amount => amount > 100 ? 0 : 5;\n');
    fs.writeFileSync(path.join(checkout, 'README.md'), '满100免运费。只改calc.cjs，保留接口。独立测试在工作区外。\n');
    const testPath = path.join(stage, 'acceptance.test.cjs');
    fs.writeFileSync(testPath, `const t=require('node:test'),a=require('node:assert/strict'),{shippingFee}=require(require('node:path').join(process.cwd(),'calc.cjs'));t('boundaries',()=>{a.equal(shippingFee(0),5);a.equal(shippingFee(99),5);a.equal(shippingFee(100),0);a.equal(shippingFee(101),0)});`);
    const request = (project, prompt) => ({ agent: 'superagent', project, prompt, model: report.model, workMode: 'guarded', thinkingLevel: 'high', timeoutSeconds: 300 });
    definitions = [
      { id: 'U01-baseline', oracle: 'readonly', original_report: text, request: request(review, '只做复核：读取 report.md，核实标记并说明哪些能力尚未验证，不修改任何业务文件。') },
      { id: 'F01-baseline', oracle: 'checkout', original_source_sha256: hash(fs.readFileSync(path.join(checkout, 'calc.cjs'))), test_path: testPath, test_sha256: hash(fs.readFileSync(testPath)), request: request(checkout, `金额正好100时仍收费，请改好并执行独立测试：node --test "${testPath}"。不要修改测试。`) },
    ];
  }
  report.frozen_definitions_sha256 = hash(JSON.stringify(definitions)); persist();
  report.planned_task_ids = definitions.map(d => d.id);
  const offset = args.includes('--offset') ? Number(option('--offset')) : 0;
  const limit = args.includes('--limit') ? Number(option('--limit')) : definitions.length;
  if (!Number.isInteger(offset) || !Number.isInteger(limit) || offset < 0 || limit < 1) throw Error('Invalid selected task range');
  const selected = definitions.slice(offset, offset + limit);
  report.selected_task_ids = selected.map(d => d.id); persist();
  for (const definition of selected) {
    await execute(definition);
    if (report.attempts.at(-1).status === 'infra_failed') break;
  }
  if (args.includes('--continuation')) {
    const previous = report.attempts.find(a => a.id === 'F01-baseline' && a.passed);
    if (!previous) throw Error('Continuation requires a verified first change');
    const sessionId = previous.events.find(e => e.kind === 'trace-snapshot')?.session_id;
    if (!sessionId) throw Error('No actual Session identity in first Run evidence');
    const current = fs.readFileSync(path.join(previous.execution_workspace, 'calc.cjs'), 'utf8');
    await execute({ id: 'U03-continuation-to-readonly', oracle: 'readonly', readonly_path: 'calc.cjs', original_report: current,
      request: { agent: 'superagent', project: previous.execution_workspace, resumeSessionId: sessionId, model: report.model,
        workMode: 'guarded', thinkingLevel: 'high', timeoutSeconds: 300,
        prompt: '只做复核：继续上一轮会话，读取当前工作副本的 calc.cjs，说明上轮修复是否保留。这次不要改代码，不要执行新的修复。' } });
    const resumed = report.attempts.at(-1);
    resumed.session_reused = resumed.events.some(e => e.kind === 'trace-snapshot' && e.session_id === sessionId);
    resumed.passed &&= resumed.session_reused;
  }
  report.selected_passed = report.attempts.length === selected.length + (args.includes('--continuation') ? 1 : 0) && report.attempts.every(a => a.passed);
  report.all_passed = report.selected_passed && selected.length === definitions.length; persist();
})().catch(error => { report.fatal = error.name + ': ' + error.message; persist(); console.error(report.fatal); process.exitCode = 1; });
