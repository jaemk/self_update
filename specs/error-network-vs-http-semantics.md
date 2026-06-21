# Error Network vs Http semantics

Status: not implemented

## Problem

`Error::Network` is raised for a non-2xx HTTP response, while `Error::Http` holds a
transport-level failure (a boxed `reqwest` / `ureq` error from the client). The naming
is counterintuitive: a reader expects "Http" to mean an HTTP status error and
"Network" to mean a connectivity failure, which is the reverse of how they are used.

## What it would take

A documentation clarification on both variants stating precisely what each means
(`Http` is a transport / client failure with the source error boxed; `Network` is a
non-success HTTP status from a request that completed). The custom-backend
`ReleaseSource` implementor docs should give the same guidance so a custom source maps
its failures to the right variant. A rename would be clearer but is a breaking change
to variant names and is out of scope here.

## Why deferred

The behavior is correct; only the names and docs are confusing. A doc clarification is
additive and low-risk but has not been written yet. A rename waits for a future
breaking window.
