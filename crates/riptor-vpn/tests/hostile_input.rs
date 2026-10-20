//! Mutated profiles: no panics, and nothing that runs programs survives.

use riptor_vpn::{mgmt, natpmp, ovpn, wg};

/// xorshift64*, seeded, so failures reproduce.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

const SNIPPETS: &[&str] = &[
    "\nup /tmp/x.sh\n",
    "\nscript-security 3\n",
    "\nplugin evil.so\n",
    "\n<ca>\n",
    "\n</ca>\n",
    "\nconfig other.ovpn\n",
    "\nmanagement 0.0.0.0 1\n",
    "\nPostUp = id\n",
    "\n[Peer]\n",
    "\"",
    "\\",
    "\r",
    "\0",
    "é",
];

fn mutate(rng: &mut Rng, base: &str) -> Vec<u8> {
    let mut b = base.as_bytes().to_vec();
    for _ in 0..1 + rng.below(4) {
        let i = rng.below(b.len() + 1).min(b.len());
        match rng.below(4) {
            0 if i < b.len() => b[i] = rng.next() as u8,
            1 => b.truncate(i),
            2 if i < b.len() => {
                b.remove(i);
            }
            _ => {
                let s = SNIPPETS[rng.below(SNIPPETS.len())];
                b.splice(i..i, s.bytes());
            }
        }
    }
    b
}

const CA: &str =
    "<ca>\n-----BEGIN CERTIFICATE-----\nMIIBszCCAVmgAwIBAgIUQ\n-----END CERTIFICATE-----\n</ca>\n";

/// Options that run programs or escape the app-only setup.
const DANGEROUS: &[&str] = &[
    "up",
    "down",
    "route-up",
    "route-pre-down",
    "ipchange",
    "tls-verify",
    "auth-user-pass-verify",
    "client-connect",
    "client-disconnect",
    "learn-address",
    "plugin",
    "management",
    "config",
    "cd",
    "chroot",
    "daemon",
    "writepid",
    "log",
    "log-append",
    "redirect-gateway",
    "setenv",
];

#[test]
fn mutated_ovpn_profiles_stay_safe() {
    let base = format!(
        "client\ndev tun\nproto udp\nremote 203.0.113.1 1194\nnobind\nauth-user-pass\nremote-cert-tls server\nscript-security 2\nup /etc/openvpn/update-resolv-conf\n{CA}key-direction 1\n"
    );
    let mut rng = Rng(0x5eed);
    let mut accepted = 0;
    for _ in 0..20_000 {
        let input = mutate(&mut rng, &base);
        let Ok(s) = ovpn::sanitize(&input) else {
            continue;
        };
        accepted += 1;
        let mut in_block = false;
        for line in s.config.lines() {
            let t = line.trim();
            if t.starts_with("</") {
                in_block = false;
                continue;
            }
            if t.starts_with('<') {
                in_block = true;
                continue;
            }
            if in_block {
                continue;
            }
            let word = t.split_whitespace().next().unwrap_or("");
            assert!(
                !DANGEROUS.contains(&word),
                "{word} survived in:\n{}",
                s.config
            );
            if word == "script-security" {
                assert!(t.ends_with(" 1") || t.ends_with(" 0"), "{t}");
            }
        }
        assert!(s.config.contains("route-nopull"));
        // The helper sanitizes again before use; that must still pass.
        assert!(ovpn::sanitize(s.config.as_bytes()).is_ok(), "{}", s.config);
    }
    assert!(accepted > 100, "only {accepted} mutations were accepted");
}

#[test]
fn mutated_wireguard_profiles_stay_safe() {
    let key = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=";
    let base = format!(
        "[Interface]\nPrivateKey = {key}\nAddress = 10.2.0.2/32\nDNS = 10.2.0.1\n\n[Peer]\nPublicKey = {key}\nAllowedIPs = 0.0.0.0/0\nEndpoint = 203.0.113.1:51820\n"
    );
    let mut rng = Rng(0xbeef);
    for _ in 0..20_000 {
        let input = mutate(&mut rng, &base);
        if let Ok(s) = wg::sanitize(&input) {
            let lower = s.config.to_ascii_lowercase();
            for hook in ["preup", "postup", "predown", "postdown", "saveconfig"] {
                assert!(!lower.contains(hook), "{hook} survived:\n{}", s.config);
            }
            assert!(s.config.contains("Table = off"));
            let _ = wg::tunnel_ipv4(&s.config);
        }
    }
}

#[test]
fn random_protocol_messages_never_panic() {
    let mut rng = Rng(42);
    for _ in 0..20_000 {
        let len = rng.below(40);
        let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let _ = natpmp::decode_response(&bytes, natpmp::Proto::Udp, 1234);
        let line = String::from_utf8_lossy(&bytes);
        let _ = mgmt::parse(&line);
        let _ = mgmt::parse(&format!(">STATE:{line}"));
        let _ = mgmt::parse(&format!(">PASSWORD:{line}"));
    }
}
