# Kithmonite[\*](https://www.thisworddoesnotexist.com/)

_Kithmonite_ is a proof of concept payment engine.

## Usage

### Quickstart

To test the system, just run the cargo command at the root and put the input file as argument.

Run the following command in the project directory:

```bash
$ cargo run -- ./transactions.csv
```

### How to cause chaos

Create a file with 10 millions transactions:

```bash
$ cd generator && cargo run > ../transactions-10m.csv -- --rows 10000000
```

Start the engine:

```bash
$ cargo run -- ./transactions-10m.csv
```

## Structure

Kithmonite follows a standard, simple structure so that a reviewer intuitively understands the role of each component.

```sh
kithmonite/
├── account         # Everything related to account and transactions
├── cli             # CLI-specific types and conversions
├── main            # Main processor process
├── processor       # Payment processor logic
└───── generator    # Generator package
```

## In retrospect

## Project completion

The work on the accuracy and reliability of the code is done. However, performance is not satisfactory, especially during deserialization.
The flame diagram below shows that discarding white space using `csv::Trim` is a severe bottleneck, it accounts for ~50% of CPU time.

<img width="100%" alt="Screen Shot 2022-03-04 at 12 08 27" src="https://user-images.githubusercontent.com/18191750/156752763-2d79eb5c-ea44-456d-a025-abb666b8e009.png">
 
## Improvements

Here are the current issues and solutions that could address them:

- Deserialization performance:
  - Use a binary serialization format like gRPC or bincode instead of CSV because it's both lighter and faster to serialize/deserialize.
  - As this is mostly a CPU-bound task, share the work accross multiple threads using a lib that supports multi-threading like rayon or tokio.
- `csv::Trim` whitespace trimming performance seems excessive for such a task. To find a workaround, I would look for solutions to do the trimming manually from the base string.
- Log the errors to the console and collect traces to debug the system if it goes into production.
- 16 bits user IDs: this limits the number of clients to 65536, maybe the identifier capacity should be 32/64bit instead.

## The generator

In order to generate transactions for load testing, the generator library can be used to create large transaction data sets. It generates low quality data, with the aim of breaking the system and discovering hidden bugs.

## Next steps

1. Debug the trimming performance
2. Distributed the serialization accross multiple threads
3. Add instrumentation to be alerted of suspicious behavior (log the errors instead of discarding them).
