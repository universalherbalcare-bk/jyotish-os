# Test fixtures — public certificates only

`san-ip-127.crt` (SAN = IP:127.0.0.1, DNS:localhost) and `san-dns-only.crt` (SAN = DNS:localhost)
are throwaway self-signed P-256 leaf certificates generated once with `openssl req -x509` for
`src/tls.rs` unit tests (`check_cert_san_pem`). Their private keys were discarded at generation
time and are not in the repository; the certificates sign nothing and trust nothing.
