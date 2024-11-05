use std::borrow::Borrow;
use std::sync::{Arc, Mutex};

use sqlx::postgres::PgPoolOptions;
use sqlx::{Pool, Postgres};
use tonic::Code;
use tonic::{transport::Server, Request, Response, Status};

use search_proto::search_server::{Search, SearchServer};
use search_proto::{SearchReleaseRequest, SearchReleaseResponse};

pub mod search_proto {
    tonic::include_proto!("search");
}

#[derive(Debug)]
struct MySearch {
    pool: Arc<Pool<Postgres>>,
}

#[tonic::async_trait]
impl Search for MySearch {
    async fn search_release(
        &self,
        request: Request<SearchReleaseRequest>,
    ) -> Result<Response<SearchReleaseResponse>, Status> {
        println!("Got a request: {:?}", request);

        let result = sqlx::query!("SELECT * FROM musicbrainz.release LIMIT 10")
            .fetch_all(self.pool.borrow())
            .await
            .map_err(|_| Status::new(Code::Internal, "Failed to query database"))?;

        println!("{result:?}");

        let reply = SearchReleaseResponse { result_count: 1 };

        Ok(Response::new(reply))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse()?;
    let database_url = std::env::var("DATABASE_URL").expect("Env `DATABASE_URL` not set!");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;

    let search = MySearch {
        pool: Arc::new(pool),
    };

    Server::builder()
        .add_service(SearchServer::new(search))
        .serve(addr)
        .await?;

    Ok(())
}
