#!/usr/bin/env python3
"""Seed a rich, diverse kyma knowledge graph for the README screenshots:
many tech-stack variants (k8s, aws, gcp, datadog, github, slack, postgres,
redis, kafka, …), services/repos/people/tables/concepts, and durable memories,
wired with realistic relationships."""
import re, subprocess, sys

KYMA = "/Users/shakedaskayo/projects/agentcylabs/kyma/target/debug/kyma"
NS = "memory/memory"
ids = {}
UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")

def run(args):
    out = subprocess.run([KYMA] + args, capture_output=True, text=True)
    m = UUID.search(out.stdout + out.stderr)
    return m.group(0) if m else None

def L(key, rel):
    u = ids.get(key)
    return f"memory:{u}|{NS}|{rel}" if u else None

def ent(key, name, kind=None, type=None, realm="acme", props=None, links=None):
    args = ["entity", name, "--realm", realm]
    if type: args += ["--type", type]
    if kind: args += ["--kind", kind]
    for k, v in (props or {}).items(): args += ["--prop", f"{k}={v}"]
    for tk, rel in (links or []):
        lk = L(tk, rel)
        if lk: args += ["--link", lk]
    uid = run(args)
    if uid: ids[key] = uid
    else: print("  ! failed:", name, file=sys.stderr)

def mem(key, content, mtype):
    uid = run(["remember", content, "--type", mtype, "--realm", "acme"])
    if uid: ids[key] = uid

print("memories…")
mem("m_idem", "Payments owns the idempotency-keys table; never duplicate it elsewhere.", "fact")
mem("m_pg", "We chose Postgres over DynamoDB for orders — transactional integrity matters more than scale here.", "decision")
mem("m_kql", "Prefer KQL over SQL for ad-hoc exploration; fall back to SQL only for vector search.", "preference")
mem("m_outage", "The 2026-05 latency spike was a missing index on orders.user_id — add indexes before launch.", "learning")
mem("m_rollback", "To roll back: scale the previous ReplicaSet up, flip the ingress, then scale the new one down.", "procedure")

print("people…")
for k, n in [("ada","Ada Lovelace"),("grace","Grace Hopper"),("linus","Linus Torvalds"),
             ("margaret","Margaret Hamilton"),("alan","Alan Turing"),("katherine","Katherine Johnson")]:
    ent(k, n, kind="person")

print("repos (github)…")
for k, n in [("r_web","acme/web"),("r_pay","acme/payments"),("r_auth","acme/auth"),
             ("r_orders","acme/orders"),("r_platform","acme/platform"),("r_notif","acme/notifications"),
             ("r_search","acme/search"),("r_infra","acme/infra")]:
    ent(k, n, type="github::repository")

print("cloud / infra…")
ent("k8s_cluster","prod-cluster", type="kubernetes::cluster")
ent("k8s_ns","prod", type="kubernetes::namespace", links=[("k8s_cluster","PART_OF")])
ent("k8s_ingress","ingress-gateway", type="kubernetes::ingress", links=[("k8s_ns","PART_OF")])
ent("k8s_web","web-pod", type="kubernetes::pod", links=[("k8s_ns","RUNS_IN")])
ent("k8s_pay","payments-pod", type="kubernetes::pod", links=[("k8s_ns","RUNS_IN")])
ent("k8s_orders","orders-deploy", type="kubernetes::deployment", links=[("k8s_ns","RUNS_IN")])
ent("aws_ec2","api-host-1", type="aws::ec2::instance")
ent("aws_rds","orders-rds", type="aws::rds::database")
ent("aws_s3","assets-bucket", type="aws::s3::bucket")
ent("aws_lambda","checkout-fn", type="aws::lambda::function")
ent("gcp_vm","analytics-vm", type="gcp::compute::instance")
ent("gcp_gke","ml-cluster", type="gcp::gke::cluster")
ent("tf","infra-modules", type="terraform::module", links=[("r_infra","LIVES_IN")])

print("data stores…")
ent("pg","orders-db", type="postgresql::database", links=[("aws_rds","HOSTED_ON")])
ent("redis","sessions-cache", type="redis::cache")
ent("kafka","events-topic", type="kafka::topic")
ent("elastic","search-index", type="elastic::index")
ent("snowflake","analytics-warehouse", type="snowflake::warehouse")
ent("mongo","profiles", type="mongodb::collection")
ent("t_orders","orders", kind="table", links=[("pg","STORED_IN")])
ent("t_users","users", kind="table", links=[("pg","STORED_IN")])
ent("t_ledger","payments_ledger", kind="table", links=[("pg","STORED_IN")])

print("services…")
ent("s_auth","auth", type="kubernetes::service", props={"tier":"critical"},
    links=[("r_auth","LIVES_IN"),("k8s_ns","RUNS_IN"),("redis","USES"),("t_users","READS")])
ent("s_pay","payments", type="kubernetes::service", props={"tier":"critical"},
    links=[("r_pay","LIVES_IN"),("k8s_pay","RUNS_ON"),("s_auth","DEPENDS_ON"),
           ("pg","USES"),("t_ledger","WRITES"),("m_idem","DOCUMENTED_BY")])
ent("s_orders","orders-api", type="kubernetes::service",
    links=[("r_orders","LIVES_IN"),("k8s_orders","RUNS_ON"),("s_auth","DEPENDS_ON"),
           ("pg","USES"),("t_orders","WRITES"),("kafka","PUBLISHES"),("m_pg","DOCUMENTED_BY")])
ent("s_web","web-frontend", type="kubernetes::service",
    links=[("r_web","LIVES_IN"),("k8s_web","RUNS_ON"),("s_pay","DEPENDS_ON"),
           ("s_auth","DEPENDS_ON"),("s_orders","DEPENDS_ON"),("aws_s3","SERVES")])
ent("s_search","search", type="kubernetes::service",
    links=[("r_search","LIVES_IN"),("elastic","USES"),("kafka","CONSUMES"),("s_orders","DEPENDS_ON")])
ent("s_notif","notifications", type="kubernetes::service",
    links=[("r_notif","LIVES_IN"),("kafka","CONSUMES"),("s_pay","DEPENDS_ON")])
ent("s_reco","recommendations", type="gcp::gke::service",
    links=[("gcp_gke","RUNS_ON"),("snowflake","READS"),("mongo","READS")])
ent("s_gw","api-gateway", type="kubernetes::service",
    links=[("k8s_ingress","ROUTES_FROM"),("s_web","ROUTES_TO"),("s_orders","ROUTES_TO"),("s_search","ROUTES_TO")])
ent("s_billing","billing", type="aws::lambda::function",
    links=[("aws_lambda","RUNS_ON"),("s_pay","DEPENDS_ON"),("snowflake","WRITES")])
ent("s_analytics","analytics-pipeline", type="gcp::compute::service",
    links=[("gcp_vm","RUNS_ON"),("kafka","CONSUMES"),("snowflake","WRITES")])

print("observability…")
ent("o_dd","orders-monitor", type="datadog::monitor", links=[("s_orders","MONITORS")])
ent("o_dd2","payments-monitor", type="datadog::monitor", links=[("s_pay","MONITORS")])
ent("o_graf","slo-dashboard", type="grafana::dashboard", links=[("s_web","MONITORS")])
ent("o_prom","latency-alert", type="prometheus::alert", links=[("s_orders","FIRES_ON"),("m_outage","DOCUMENTED_BY")])
ent("o_sentry","error-tracker", type="sentry::issue", links=[("s_web","TRACKS")])
ent("o_otel","trace-checkout", type="opentelemetry::trace", links=[("s_pay","TRACES")])
ent("o_pd","oncall", type="pagerduty::service", links=[("o_dd","ESCALATES"),("o_dd2","ESCALATES")])

print("collaboration…")
ent("c_slack","#incidents", type="slack::channel", links=[("o_pd","NOTIFIES")])
ent("c_jira","ORD-412", type="jira::issue", links=[("s_orders","ABOUT")])
ent("c_linear","KYMA-45", type="linear::issue", links=[("s_search","ABOUT")])
ent("c_conf","orders runbook", type="confluence::page", links=[("s_orders","DOCUMENTS"),("m_rollback","DOCUMENTED_BY")])
ent("c_notion","payments design", type="notion::page", links=[("s_pay","DOCUMENTS")])

print("ownership + concepts…")
ent("ada", "Ada Lovelace", kind="person", links=[("s_pay","OWNS"),("s_auth","OWNS"),("o_pd","MEMBER_OF")])
ent("grace", "Grace Hopper", kind="person", links=[("s_orders","OWNS"),("s_search","OWNS")])
ent("linus", "Linus Torvalds", kind="person", links=[("s_web","OWNS"),("tf","OWNS")])
ent("margaret", "Margaret Hamilton", kind="person", links=[("s_notif","OWNS"),("s_billing","OWNS")])
ent("alan", "Alan Turing", kind="person", links=[("s_reco","OWNS"),("s_analytics","OWNS")])
ent("cx", "checkout-flow", kind="concept", links=[("s_web","INVOLVES"),("s_pay","INVOLVES"),("s_orders","INVOLVES"),("aws_lambda","INVOLVES")])
ent("cx2", "oncall-rotation", kind="concept", links=[("o_pd","INVOLVES"),("ada","INVOLVES"),("grace","INVOLVES")])
ent("cx3", "data-retention", kind="concept", links=[("snowflake","INVOLVES"),("pg","INVOLVES"),("m_pg","INVOLVES")])

print("done. total nodes:", len(ids))
