#! /bin/bash -
tmpdir=$(mktemp -d) || exit
trap 'rm -rf "$tmpdir"' EXIT INT TERM HUP

FIFO=$tmpdir/FIFO
mkfifo "$FIFO" || exit

conf=$tmpdir/conf

cat > "$conf" << 'EOF' || exit
split -v
focus
screen -t stderr sh -c 'tty > "$FIFO"; read done < "$FIFO"'
focus
screen -t stdout sh -c 'read tty < "$FIFO"; eval "cargo run -- $CMD" 2> "$tty"; echo "[Command exited with status $?, press enter to exit]"; read prompt; echo done > "$FIFO"'
EOF

CMD="$*"
export FIFO CMD RUST_LOG=p2pclient=debug

screen -mc "$conf"