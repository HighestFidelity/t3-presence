# Runs on a linked machine over SSH, sent inline by the app. Installs nothing, writes nothing.
# Prints one JSON line of T3 state every 15 s from its state DB, opened read-only. Exits when the SSH link goes.
# Only counts, model choices, item kinds and the project title: never thread titles, messages, paths or commands.
import json, os, sqlite3, time

P = "orchestration_v2_projection_"
DB = os.path.expanduser("~/.t3/userdata/statev2.sqlite")
LIVE = "from " + P + "turn_items i where i.run_id in (select run_id from " + P + "runs where status='running')"


def read():
    c = sqlite3.connect("file:" + DB + "?mode=ro", uri=True, timeout=1.5)
    try:
        one = lambda q: c.execute(q).fetchone()[0]
        o = {
            "running": one("select count(*) from " + P + "runs where status='running'"),
            "subagents": one("select count(*) from " + P + "subagents where status='running'"),
            "sessions": one("select count(*) from " + P + "provider_sessions where status='ready'"),
            "threads": one("select count(*) from " + P + "threads where archived_at is null and deleted_at is null"),
            "selections": [r[0] for r in c.execute("select json_extract(payload_json,'$.modelSelection') from " + P + "runs where status='running'")],
            "projects": [r[0] for r in c.execute("select coalesce(p.title,'No project') from " + P + "runs r join " + P + "threads t on t.thread_id=r.thread_id left join projection_projects p on p.project_id=t.project_id where r.status='running' group by 1 order by max(r.requested_at) desc")],
        }
        r = c.execute("select json_extract(r.payload_json,'$.modelSelection'), coalesce(r.completed_at,r.requested_at), coalesce(p.title,'No project') from " + P + "runs r join " + P + "threads t on t.thread_id=r.thread_id left join projection_projects p on p.project_id=t.project_id order by r.requested_at desc limit 1").fetchone()
        if r:
            o["lastSelection"], o["lastAt"], o["project"] = r
        r = c.execute("select json_extract(payload_json,'$.tokenUsage.usedTokens'), json_extract(payload_json,'$.tokenUsage.maxTokens') from " + P + "provider_turns where provider_turn_id in (select provider_turn_id from " + P + "provider_turns order by started_at desc limit 40) and json_extract(payload_json,'$.tokenUsage.usedTokens') is not null order by started_at desc limit 1").fetchone()
        if r:
            o["ctxUsed"], o["ctxMax"] = r
        if o["running"] > 0:
            o["waiting"] = one("select count(*) " + LIVE + " and i.status='running' and (i.type='user_input_request' or (i.type='dynamic_tool' and json_extract(i.payload_json,'$.toolName')='AskUserQuestion'))") > 0 or one("select count(*) from " + P + "runtime_requests where status not in ('resolved','cancelled','expired','failed') and thread_id in (select thread_id from " + P + "runs where status='running')") > 0
            r = c.execute("select i.type, case when i.type='dynamic_tool' then json_extract(i.payload_json,'$.toolName') end, i.status, i.updated_at " + LIVE + " and i.type not in ('notification','system_notice','run_interrupt_request','run_interrupt_result','user_input_request') order by i.updated_at desc limit 1").fetchone()
            if r:
                o["liveType"], o["liveTool"], o["liveStatus"], o["liveAt"] = r
        return o
    finally:
        c.close()


while True:
    try:
        out = read()
    except Exception as e:
        out = {"error": str(e)[:200]}
    print(json.dumps(out), flush=True)
    time.sleep(15)
