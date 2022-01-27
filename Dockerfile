FROM rust:1.58

COPY . .
WORKDIR ./kollider-hedge

RUN cargo install --path .

CMD ["kollider-hedge"]
