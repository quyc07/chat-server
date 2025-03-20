CREATE TABLE "friend_ship"
(
    id        int auto_increment
        primary key,
    user_id_1 int                                not null,
    user_id_2 int                                not null,
    c_time    datetime default CURRENT_TIMESTAMP not null,
    constraint friend_ship_user_id_1_user_id_2_uindex
        unique (user_id_1, user_id_2)
)