# Roadmap

What we're working on and exploring next. Nothing on this page is a
commitment, and there are no dates. Items move to the
[changelog](../CHANGELOG.md) when they ship.

Have a use case or a priority? Tell us in
[GitHub Discussions](https://github.com/kerniqq/kerniq-server/discussions).

## Exploring

### kerniq-ai

A standalone library for building AI-powered Rust applications through one
consistent API, without learning the details of every model provider,
vector database or protocol. It would cover:

- **Model APIs:** chat and completion across providers behind one interface.
- **Chat client:** conversations, streaming and structured output.
- **Embeddings and vector stores:** storing and searching embeddings for
  retrieval-augmented generation (RAG).
- **Tool calling:** expose Rust functions to models as tools.
- **MCP:** connect to and serve tools over the Model Context Protocol.

kerniq-ai is planned as a **separate project, not part of Kerniq Server**.
It should work with any Rust web framework, including Kerniq, Axum, Actix
Web, Rocket and Poem, or with no web framework at all. Kerniq will provide a
first-class integration.

It is at the design stage. The most useful input right now:

- which model providers and vector stores you need first;
- whether you'd use MCP mainly as a client, a server, or both;
- what you use today (and what you'd want done differently).

Share it in the
[kerniq-ai discussion](https://github.com/kerniqq/kerniq-server/discussions/9).
