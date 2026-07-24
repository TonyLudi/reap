use reap_durable_writer::DurableReservation;

fn replay(reservation: DurableReservation<u8>) {
    let _first = reservation.clone();
    let _second = reservation;
}

fn main() {}
