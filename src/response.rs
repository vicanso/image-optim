use crate::error::HTTPError;

pub type ResponseResult<T> = Result<T, HTTPError>;
