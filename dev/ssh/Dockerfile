# An SSH bastion for testing tunnels: key-only login as `rift`, TCP forwarding on, port 2222.
FROM alpine:3.20
RUN apk add --no-cache openssh-server \
 && ssh-keygen -A \
 && adduser -D rift \
 && sed -i 's/^rift:!/rift:*/' /etc/shadow \
 && printf 'Port 2222\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nPermitRootLogin no\nAllowTcpForwarding yes\nStrictModes no\nAuthorizedKeysFile /keys/authorized_keys\n' > /etc/ssh/sshd_config
CMD ["/usr/sbin/sshd", "-D", "-e"]
