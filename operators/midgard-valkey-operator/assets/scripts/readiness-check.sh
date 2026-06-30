#!/bin/sh
set -e

timeout=${1:-"1"}
port=${2:-"6379"}

timeout_cmd() {
    duration=$1; shift
    "$@" &
    cmdpid=$!

    count=0
    max_count=$((duration * 10))
    while [ $count -lt $max_count ]; do
        if ! kill -0 $cmdpid 2>/dev/null; then
            wait $cmdpid
            return $?
        fi
        sleep 0.1
        count=$((count + 1))
    done

    kill -TERM $cmdpid 2>/dev/null
    sleep 0.1
    kill -0 $cmdpid 2>/dev/null && sleep 1 && kill -KILL $cmdpid 2>/dev/null
    wait $cmdpid 2>/dev/null
    return 124
}

tls_args=""
if [ -n "${VALKEY_TLS_ARGS:-}" ]; then
    tls_args="$VALKEY_TLS_ARGS"
fi

response=$(
    timeout_cmd $timeout \
    valkey-cli -h localhost -p $port $tls_args PING)

if [ "$response" != "PONG" ]; then
    echo "$response" >&2
    exit 1
fi
