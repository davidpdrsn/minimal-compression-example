# tower-http compression + tonic streams

This repo contains a minimal reproducible example of [tower-http#81](https://github.com/tower-rs/tower-http/issues/81) which is that applying compression and decompression middlewares makes tonic streams hang forever when await and item.

`cargo test` should reproduce the error.
