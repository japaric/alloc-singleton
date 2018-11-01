set -euxo pipefail

main() {
    cargo test

    if [ $TRAVIS_RUST_VERSION = nightly]; then
        cargo test --features nightly
    fi
}

main
