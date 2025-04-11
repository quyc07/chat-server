create table main.friend_ship
(
    id        integer                            not null constraint id
            primary key autoincrement,
    user_id_1 int                                not null,
    user_id_2 int                                not null,
    c_time    datetime default CURRENT_TIMESTAMP not null,
    constraint user_rel_user_id_1_user_id_2_uindex
        unique (user_id_1, user_id_2)
);