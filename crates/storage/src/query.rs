use sea_orm::{ConnectionTrait, DbBackend, DbErr, ExecResult, QueryResult, Statement, Value};

pub(crate) fn statement(sql: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DbBackend::Postgres, sql, values)
}

pub(crate) async fn exec(
    conn: &impl ConnectionTrait,
    sql: &str,
    values: Vec<Value>,
) -> Result<ExecResult, DbErr> {
    conn.execute_raw(statement(sql, values)).await
}

pub(crate) async fn one(
    conn: &impl ConnectionTrait,
    sql: &str,
    values: Vec<Value>,
) -> Result<Option<QueryResult>, DbErr> {
    conn.query_one_raw(statement(sql, values)).await
}

pub(crate) async fn all(
    conn: &impl ConnectionTrait,
    sql: &str,
    values: Vec<Value>,
) -> Result<Vec<QueryResult>, DbErr> {
    conn.query_all_raw(statement(sql, values)).await
}
