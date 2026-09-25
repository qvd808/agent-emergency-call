# Check-in agent

A voice assistant that holds a short spoken check-in with a resident, and hands anything
worrying to a human.

## Language

### People

**Resident**:
The person who lives alone and is being checked on, whichever side placed the call.
_Avoid_: caller, callee, user, patient, senior

**Dispatcher**:
The human who receives escalations, missed check-in alerts and concern flags. Never the police
or an emergency service.
_Avoid_: operator, emergency services, 911

**Bystander**:
Anyone other than the resident whose voice reaches a check-in from the resident's end of the
line, such as a visiting grandchild or a television.
_Avoid_: caller, third party

### Calls

**Check-in**:
One short spoken conversation in which the agent asks whether the resident feels okay, has eaten
today, has had any falls or pain, and needs anything.
_Avoid_: welfare call, wellness call

**Check-in attempt**:
One try at reaching the resident for a scheduled check-in. An attempt is missed when, for any
reason, it does not reach a check-in.
_Avoid_: retry, dial

### Handing off to a human

**Escalation**:
Handing a live check-in to the dispatcher so that a human is on the line now: because the
resident may be in danger, stops answering, asks for a person, or because the agent can no
longer carry on the check-in. The agent says it is connecting the resident to someone, tells
them to call 911 themselves if they are in danger, then transfers the call.
_Avoid_: transfer (that is only the last step), emergency call

**Missed check-in alert**:
A notice to the dispatcher that every attempt at a scheduled check-in was missed. There is no
live call to hand over.
_Avoid_: emergency alert, no-answer transfer

**Concern flag**:
A notice to the dispatcher, with a summary, that a check-in raised something a human should
know about, but that can wait for a callback rather than needing someone on the line now.
_Avoid_: warning, alert
