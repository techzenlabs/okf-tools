---
type: "Reference"
title: "Planted"
---

-----BEGIN RSA PRIVATE KEY-----
VGhpcyBpcyBhIHN5bnRoZXRpYyBmaXh0dXJlLCBub3QgYSBrZXkuIElmIHlvdSBhcmUgcmVhZGlu
ZyBpdCBiZWNhdXNlIGEgc2Nhbm5lciBmaXJlZCwgdGhlIHNjYW5uZXIgd29ya2VkLg==
-----END RSA PRIVATE KEY-----

Synthetic: the base64 above decodes to a sentence saying it is a fixture. The
header sits on line 6 on purpose — the finding this file exists to force is
`alpha/planted.md:6: private-key`, and the check asserts the line number so a
scanner that drifts to reporting the wrong place goes red too.
