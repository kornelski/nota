# Nota: Not a pragmatic message format

Nota is a basic serialization format which doesn't concern itself with interoperability with most programming languages. It's almost as compact as [MsgPack](https://lib.rs/crates/rmp) and [CBOR](https://lib.rs/ciborium), but lacks most features of the more mature alternatives. Nota is not well suited for high-performance parsing or serialization.

Nota uses [its own](https://www.crockford.com/kim.html) string encoding, which isn't UTF-8. Nota calls the binary float format used by literally every CPU and GPU today "*obsolete*", and uses its own decimal-in-binary bignum format instead. Conversion of standard IEEE754 floats to Nota's representaion is surprisingly difficult to perform losslessly, and it's a process as inefficient as conversion of floats to strings.

Nota's design prominently features a concept of continuation bits, but uses them inconsistently. Unlike terminators in CBOR, the continuation bits don't nest, so Nota has to fall back to `length+data` approach, which prevents streaming serialization. It's impossible to skip over records without parsing them. String lengths are in codepoints, not bytes, so they can't be accurately preallocated ahead of time or skipped over without parsing, even if using Nota's own string encoding instead of UTF-8.

## Not a JSON

The [spec](https://www.crockford.com/nota.html) defines only a subset of JSON's types, and does not define how to deal with the rest of them.

## Not a maintained project

This was an experiment to see if Nota was noteworthy, and it wasn't. The first person to file a bug is the new maintainer.
