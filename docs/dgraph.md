# 语法

## 修改

```json
{
  "set": [
    {
      "name": "Bob",
      "phone": "12345678"
    }
  ]
}
```
```json
{
  "data": {
    "code": "Success",
    "message": "Done",
    "queries": null,
    "uids": {
      "dg.3962518893.1": "0x4e2a"
    }
  },
  "extensions": {
    "server_latency": {
      "parsing_ns": 178959,
      "processing_ns": 2809084,
      "assign_timestamp_ns": 1063917,
      "total_ns": 4325958
    },
    "txn": {
      "start_ts": 20158,
      "commit_ts": 20159,
      "preds": [
        "1-0-name",
        "1-0-phone"
      ]
    }
  }
}
```

## 查询
```
{
  users(func: has(name)) {
  uid
  name
  phone
}
}
```
```json
{
  "data": {
    "code": "Success",
    "message": "Done",
    "queries": null,
    "uids": {
      "dg.3962518893.1": "0x4e2a"
    }
  },
  "extensions": {
    "server_latency": {
      "parsing_ns": 178959,
      "processing_ns": 2809084,
      "assign_timestamp_ns": 1063917,
      "total_ns": 4325958
    },
    "txn": {
      "start_ts": 20158,
      "commit_ts": 20159,
      "preds": [
        "1-0-name",
        "1-0-phone"
      ]
    }
  }
}
```
## 建立关系
```json
 {
  "set": [
    {
      "uid": "0x4e2d",
      "friend": [
        {
          "uid": "0x4e37"
        }
      ]
    }
  ]
}
```