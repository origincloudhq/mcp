# Rules for working with Origin Cloud

These hold on every surface: the `origin` CLI, Terraform, the SDKs and the
MCP tools.

## Credentials

- Work as the login the user already has: the `origin` CLI's session and
  profiles, or the credentials the environment holds (`ORIGINCLOUD_*`
  variables, a CI job's identity). Use the profile the user names; don't
  switch profiles or log in as someone else on your own.
- Never print, log, echo or write a token, client secret or private key:
  not in a reply, a command's output you show, a file, a commit or a
  configuration file. Don't ask the user to paste one into the
  conversation. A configuration you write names a profile or an
  environment variable, never a credential.
- A call that answers with a credential (a service account's secret, a
  generated private key) is the user's to run: tell them the command and
  where its output should go, and don't read the output yourself.

## Read before you change

- Before changing or deleting a resource, read it as it is now and base the
  change on that, not on what you expect it to be.
- Work in the project, zone and region the user works in: read the current
  context or ask. Don't guess one, and don't fall back to another.

## Ask before deletes and one-way changes

- A delete, and a change that can't be undone (growing a volume, which
  never shrinks back, is one), needs the user's yes for that resource in
  this conversation. Say what will change and what is lost, then wait.
- A yes covers what it named, nothing more. Never delete, replace or
  recreate something to get past an error.

## Wait on operations

- A create, update or delete answers with an operation. The change is done
  when the operation is `done`: then it holds the result (`response`) or
  the `error`. Wait for it before using the result, starting what depends
  on it, or telling the user it worked.
- A retried create or change carries the same `request_id` as the first
  attempt, so that it can't happen twice.

## Names

- A resource is identified by its full resource name: its service's host
  and its relative name, such as
  `//pvs.origincloudapis.eu/projects/my-project/zones/eu-central1-az1/volumes/vol-1a2b3c`.
  The resource's `name` field is the relative part,
  `projects/my-project/zones/eu-central1-az1/volumes/vol-1a2b3c`.
- A reference to another resource, an IAM binding's member and an audit
  entry's subject are full names: give the whole name, never a bare id, an
  email address or a display name.
- Take names from what the API answered; don't build them from display
  names, and don't expect a URN.

## Errors

- Report a failure as the API sent it: its code (the `reason`, such as
  `NOT_FOUND` or `CONFLICT`), its message, and the correlation ID (the
  `requestId` of its `RequestInfo`), so that support can find the call.
  Don't paraphrase it into something vaguer.
- Don't repeat a refused call unchanged. Retry only what the error says is
  retryable, with the same `request_id`; when unsure whether a change took
  effect, read the resource first.
