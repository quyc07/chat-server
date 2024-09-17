CREATE TABLE IF NOT EXISTS "friend_request"
(
    id          integer not null
        constraint friend_request_pk
            primary key autoincrement,
    request_id  integer not null,
    target_id   integer not null,
    reason      varchar(300),
    create_time datetime default CURRENT_TIMESTAMP,
    modify_time datetime
    , status varchar(10) default 'WAIT' not null);

CREATE TABLE IF NOT EXISTS "group"
(
    id     integer                            not null
        constraint group_pk
            primary key autoincrement,
    name   varchar(300)                       not null,
    admin  integer                            not null,
    c_time datetime default CURRENT_TIMESTAMP not null,
    u_time datetime
);

CREATE TABLE IF NOT EXISTS "read_index"
(
    id                integer not null
        constraint read_index_pk
            primary key autoincrement,
    uid               integer not null
        constraint read_index_user_id_fk_2
            references user,
    target_uid        integer
        constraint read_index_user_id_fk
            references user,
    target_gid        integer
        constraint read_index_group_id_fk
            references "group",
    mid               integer,
    latest_mid        integer not null,
    uid_of_latest_msg integer not null
);

CREATE TABLE IF NOT EXISTS "user"
(
    id          integer                               not null
        constraint user_pk
            primary key autoincrement,
    name        varchar(255)                          not null,
    phone       varchar(11),
    password    varchar(255)                          not null,
    create_time datetime    default CURRENT_TIMESTAMP not null,
    update_time datetime,
    status      varchar(10) default 'NORMAL'          not null,
    dgraph_uid  varchar(10) default ''                not null,
    role        varchar(10) default 'User'            not null
    , email varchar(300));

CREATE TABLE IF NOT EXISTS "user_group_rel"
(
    id       integer                            not null
        constraint user_group_rel_pk
            primary key autoincrement,
    group_id integer                            not null,
    user_id  integer                            not null,
    c_time   datetime default CURRENT_TIMESTAMP not null,
    forbid   boolean  default false             not null
);

