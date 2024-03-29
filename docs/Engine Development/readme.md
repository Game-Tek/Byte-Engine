## Testing
### Run tests
To run the tests use the following command:
```bash
cargo test
```

This will run _all_ tests in the engine. But sometime you may wish to skip some tests, specially the rendering ones. To do this you can use the `--skip` flag. For example, to skip the rendering tests you can run:
```bash
cargo test -- --skip render --skip graphics
```

### Code coverage
To generate test code coverage we recommend using the `cargo-llvm-cov` crate. To install it run:
```bash
cargo install cargo-llvm-cov
```

Then, to run the tests and generate a coverage report file run:

```bash
cargo llvm-cov --lcov --output-path coverage/lcov.info
```

To see the coverage report inline in VSCode you can use the `markis.code-coverage` extension.