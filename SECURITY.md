# Security

A security bug in loft is handled as a bug: fixed through the normal process and shipped in
the next release. What differs is when we say why it mattered: one week after the release
that fixes it is out, so there is time to install the fix first. Nothing stays unpublished:
hiding a defect does not fix it.

## Reporting one

Please do not open a public issue for something that could be used against people running
loft programs. Use **Security → Report a vulnerability** on this repository, which reaches
the maintainers privately. That is also the place for a defect in one of the published
libraries (`loft-libs-*`): they are maintained here, and this policy covers them. A library
maintained by someone else names its own contact. Say what you ran and what happened; a small program or file that
shows it is the most useful thing you can send.

You will get an answer, and you will be told when the fix is released. You are credited in
the published report unless you ask not to be.

## What happens next

1. The report is recorded privately and reproduced.
2. It is fixed as an ordinary change, with a regression test, and merged the usual way.
3. It ships in the next loft release, or the next version of the library it is in.
4. One week after that release can be installed, the report is published in this tracker as
   a closed issue that names the fixing commit and the release, together with an in-depth
   review of the batch it belongs to: what was examined, what was found and fixed, what was
   found sound, and what is still open.

When a published library version is unsafe to keep using, the package registry's advisory
feed names it and the version that fixes it; `loft install` and a `--ship` build then say so.

A defect that is being used against people, or that anyone on the network can reach in a
shipped program, does not wait for the regular release.

## What to expect

There is no embargo list and no announcement schedule. We do not request a CVE unless
someone downstream asks for one. Every finding is published a week after its fix is
installable, including the ones we find ourselves.

## Supported versions

The latest release. Fixes are not back-ported to earlier ones.
