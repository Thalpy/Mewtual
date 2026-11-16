# Native browser-consumer startup acceptance

This is a test-fixture correction. It changes no production protocol deadline,
attempt, permission or application startup code. It does not establish Chat V2,
physical WebView/RTC, audible media or public-network acceptance.

At PR95 source8bca2d8d, Linux complete native execution passed. Windows first
failed in `native_browser_maximum_offer_answer_round_uses_real_http_and_blocked_tcp_bridge`
while pumping automatic confirmation, before SDP. The37 later failures were
intentional refusals by the shared fixture after unsuccessful cleanup; they were
not independent failures. The new fix retains that isolation policy.

The fixture previously started native service maintenance before spawning Node,
then waited for both endpoint preparations and remote discovery before starting
either local browser consumer. Production starts its consumer immediately after
authenticated setup. Native pair work has an original finite deadline. Spending
that deadline inside a fixture-only interval with no local consumer is a real
ordering mismatch. Expiry as the specific cause of the recorded Windows failure
remains a hypothesis: its old timeout diagnostics did not expose the missing
confirmation child or original expiry.

The closed automatic-startup mode now parks the original service's first
maintenance turn before process launch, captures the actual Session's sole native
factory hold at Setup, and retains both until the same endpoint makes a successful
authenticated ReadWork through the real command adapter. It drops the factory hold
first, then the cadence hold. Each endpoint releases independently. It waits for
no remote endpoint, supplies no parent or role to the browser, and renews no
deadline, action, permit or attempt. Refused reads do not release the holds;
historical same-owner reads may do so, but cannot reopen an ended Session.

The automatic confirmation scenarios assert both actual holds exist and no
original Derive has been admitted after genuine counterpart discovery, before
starting the consumers. Their existing terminal receipt, exact ACK, settlement,
Return, Ready, SDP and real blocked-TCP bridge checks stay unchanged. Manual
confirmation, End-before-browser-input and deferred local-refusal scenarios
retain their intentional earlier order. Collection always releases test holds.

Failure diagnostics retain only the discovered pair's original public selection
for observation. This does not scope terminal success or feed browser/native
admission. A bounded32-row try-lock snapshot reports confirmation role, attachment,
ACK, drainage, engine settlement and outcome class, with formatting after guards
drop. It carries no key, plaintext, frame payload or action owner.

Local Windows execution of the formerly failing maximum-SDP scenario passes twice.
Ten focused real native cases plus the six ACL controls pass; full
current-source Root/native/frontend CI remains pending. Read-only actual-diff
review found no blocker/high/medium. Remaining coverage limits: no deterministic
held-second-Setup control of the old order; the child snapshot does not expose
original deadline/expiry or Ready-stage state. Remote-peer delay can still expire
an original protocol exchange; this fixture does not claim otherwise.

The separate native `cargo check`, native formatting, ambient and whitespace
checks also pass. No production setup, runtime placement or UI flow changed, so
the application-startup and browser-flow gates are not implicated by this diff.
